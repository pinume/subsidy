use std::collections::{HashMap, HashSet};

/// 候选值优先判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PriorityOutcome {
    Unique(String),
    Ambiguous,
    NoHit,
}

impl PriorityOutcome {
    /// 对一组候选值去重：恰好一个不同值→`Unique`；两个及以上→`Ambiguous`；没有候选→`NoHit`。
    pub(crate) fn resolve(hits: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut distinct = HashSet::new();
        for hit in hits {
            distinct.insert(hit.into());
            if distinct.len() > 1 {
                return Self::Ambiguous;
            }
        }
        match distinct.into_iter().next() {
            Some(unique) => Self::Unique(unique),
            None => Self::NoHit,
        }
    }

    /// 解包为唯一值；若是歧义则累加计数器并返回 None；若是无命中返回 None。
    pub(crate) fn into_unique(self, ambiguous_counter: &mut usize) -> Option<String> {
        match self {
            Self::Unique(value) => Some(value),
            Self::Ambiguous => {
                *ambiguous_counter += 1;
                None
            }
            Self::NoHit => None,
        }
    }
}

/// `字段值 → 该值下全部候选（未去重）`，封装歧义判定与唯一值提取；
/// 用于按某个匹配键在另一数据源中查找唯一命中值的场景（如销售用券情况统计第10.10/
/// 10.12节、已上传数据与回款明细的匹配）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MultiValueIndex {
    map: HashMap<String, Vec<String>>,
}

impl MultiValueIndex {
    pub(crate) fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.map.entry(key.into()).or_default().push(value.into());
    }

    pub(crate) fn resolve(&self, key: Option<&str>) -> PriorityOutcome {
        let Some(key) = key.filter(|k| !k.is_empty()) else {
            return PriorityOutcome::NoHit;
        };
        let Some(hits) = self.map.get(key) else {
            return PriorityOutcome::NoHit;
        };
        PriorityOutcome::resolve(hits.iter().cloned())
    }

    #[cfg(test)]
    pub(crate) fn get(&self, key: &str) -> Option<&[String]> {
        self.map.get(key).map(|v| v.as_slice())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_outcome_resolves_unique_ambiguous_and_no_hit() {
        assert_eq!(
            PriorityOutcome::resolve(Vec::<String>::new()),
            PriorityOutcome::NoHit
        );
        assert_eq!(
            PriorityOutcome::resolve(vec!["A"]),
            PriorityOutcome::Unique("A".into())
        );
        assert_eq!(
            PriorityOutcome::resolve(vec!["A", "A"]),
            PriorityOutcome::Unique("A".into())
        );
        assert_eq!(
            PriorityOutcome::resolve(vec!["A", "B"]),
            PriorityOutcome::Ambiguous
        );
    }

    #[test]
    fn into_unique_accumulates_ambiguous_counter() {
        let mut count = 0;
        assert_eq!(
            PriorityOutcome::Unique("OK".into()).into_unique(&mut count),
            Some("OK".into())
        );
        assert_eq!(count, 0);

        assert_eq!(PriorityOutcome::Ambiguous.into_unique(&mut count), None);
        assert_eq!(count, 1);

        assert_eq!(PriorityOutcome::NoHit.into_unique(&mut count), None);
        assert_eq!(count, 1);
    }

    #[test]
    fn multi_value_index_resolves_keys_and_handles_empty_key() {
        let mut index = MultiValueIndex::new();
        index.insert("DOC1", "INV1");
        index.insert("DOC1", "INV1"); // duplicate identical value
        index.insert("DOC2", "INV2");
        index.insert("DOC2", "INV3"); // ambiguous
        index.insert("DOC3", "INV4");

        assert_eq!(index.len(), 3);
        assert!(!index.is_empty());

        assert_eq!(
            index.resolve(Some("DOC1")),
            PriorityOutcome::Unique("INV1".into())
        );
        assert_eq!(index.resolve(Some("DOC2")), PriorityOutcome::Ambiguous);
        assert_eq!(
            index.resolve(Some("DOC3")),
            PriorityOutcome::Unique("INV4".into())
        );
        assert_eq!(index.resolve(Some("NOT_EXIST")), PriorityOutcome::NoHit);
        assert_eq!(index.resolve(Some("")), PriorityOutcome::NoHit);
        assert_eq!(index.resolve(None), PriorityOutcome::NoHit);

        assert_eq!(
            index.get("DOC1"),
            Some(&["INV1".to_string(), "INV1".to_string()][..])
        );
        assert_eq!(index.get("NONE"), None);
    }
}
