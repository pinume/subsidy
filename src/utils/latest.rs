use std::collections::HashMap;
use std::hash::Hash;

/// 清洗保留重复导出（如审核失败后重新提交）：同一非空键只保留时间最新的项，最新时间相同的项都保留，
/// 由调用方按各自规则判定歧义或冲突；没有时间视为最旧；`key` 返回 `None`（空键）的项全部保留。
/// 清洗（第10.12.2节）与报表（第4.1.1节、财务统筹表）共用此规则。按原顺序返回。
pub fn latest_by_key<'a, T, K, Tm>(
    items: &'a [T],
    key: impl Fn(&'a T) -> Option<K>,
    time: impl Fn(&'a T) -> Option<Tm>,
) -> Vec<&'a T>
where
    K: Eq + Hash,
    Tm: Ord,
{
    let mut latest: HashMap<K, Option<Tm>> = HashMap::new();
    for item in items {
        if let Some(k) = key(item) {
            let current = latest.entry(k).or_insert(None);
            let candidate = time(item);
            if candidate > *current {
                *current = candidate;
            }
        }
    }
    items
        .iter()
        .filter(|item| key(item).is_none_or(|k| latest[&k] == time(item)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_latest_per_key_ties_and_empty_keys_in_order() {
        // (键, 时间, 标记)
        let items = [
            (Some("A"), Some(1), "a-old"),
            (Some("A"), Some(2), "a-new"),
            (Some("B"), Some(1), "b-tie-1"),
            (Some("B"), Some(1), "b-tie-2"),
            (Some("C"), None, "c-untimed"),
            (Some("C"), Some(0), "c-timed"),
            (None, None, "no-key-1"),
            (None, Some(5), "no-key-2"),
        ];
        let kept: Vec<_> = latest_by_key(&items, |item| item.0, |item| item.1)
            .into_iter()
            .map(|item| item.2)
            .collect();
        assert_eq!(
            kept,
            [
                "a-new", "b-tie-1", "b-tie-2", "c-timed", "no-key-1", "no-key-2"
            ]
        );
    }
}
