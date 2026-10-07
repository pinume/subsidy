use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use super::{Category, MultiValueIndex, invoice, receipts, unionpay};
use crate::model::{ProcessError, Table, Value};

/// 命中分类互斥；不同阶段的歧义可重复计数。
#[derive(Default, Clone, Copy)]
pub(crate) struct MatchStats {
    pub(super) hits: [usize; 4],
    pub(super) ambiguous: [usize; 4],
}

#[derive(Default)]
struct RunCache {
    tables: HashMap<Category, Table>,
    authority: Option<Rc<HashSet<String>>>,
    invoices: Option<Rc<MultiValueIndex>>,
    receipts: Option<Rc<MultiValueIndex>>,
    stats: Option<MatchStats>,
}

thread_local! {
    static CACHE: RefCell<Option<RunCache>> = const { RefCell::new(None) };
}

pub(crate) struct ScopedCache;

impl ScopedCache {
    pub(crate) fn activate() -> Self {
        CACHE.with(|c| *c.borrow_mut() = Some(RunCache::default()));
        ScopedCache
    }

    pub(crate) fn get(category: Category) -> Option<Table> {
        CACHE.with(|c| c.borrow().as_ref()?.tables.get(&category).cloned())
    }

    /// 只读借用缓存中的表并执行闭包，避免整表深拷贝。
    pub(crate) fn with_table<F, R>(category: Category, f: F) -> Option<R>
    where
        F: FnOnce(&Table) -> R,
    {
        CACHE.with(|c| {
            let borrow = c.borrow();
            let table = borrow.as_ref()?.tables.get(&category)?;
            Some(f(table))
        })
    }

    pub(crate) fn put(category: Category, table: &Table) {
        CACHE.with(|c| {
            if let Some(map) = c.borrow_mut().as_mut() {
                map.tables.insert(category, table.clone());
            }
        });
    }

    pub(crate) fn authority(input_dir: &Path) -> Result<Rc<HashSet<String>>, ProcessError> {
        if let Some(index) = CACHE.with(|c| c.borrow().as_ref()?.authority.clone()) {
            return Ok(index);
        }
        let records = unionpay::load_records(input_dir)?;
        Ok(CACHE
            .with(|c| c.borrow().as_ref()?.authority.clone())
            .unwrap_or_else(|| Self::cache_authority(&records)))
    }

    fn cache_authority(records: &[unionpay::UnionPayRecord]) -> Rc<HashSet<String>> {
        let index = Rc::new(
            records
                .iter()
                .map(|r| r.retrieval_no.as_str())
                .filter(|v| is_valid_ref_no(v))
                .map(str::to_owned)
                .collect(),
        );
        CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().as_mut() {
                cache.authority = Some(Rc::clone(&index));
            }
        });
        index
    }

    /// 按`匹配单据号`汇总发票明细全部非空`数电发票号码`（未去重）；歧义判定见`coupons::to_row`
    /// 中复用的`resolve`（与 10.6.2 节"命中权威值需唯一"同一原则，不得任选）。
    pub(crate) fn invoice_index(input_dir: &Path) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if let Some(index) = CACHE.with(|c| c.borrow().as_ref()?.invoices.clone()) {
            return Ok(index);
        }
        let records = invoice::load_records(input_dir)?;
        Ok(CACHE
            .with(|c| c.borrow().as_ref()?.invoices.clone())
            .unwrap_or_else(|| Self::cache_invoice_index(&records)))
    }

    fn cache_invoice_index(records: &[invoice::InvoiceRecord]) -> Rc<MultiValueIndex> {
        let mut grouped: MultiValueIndex = HashMap::new();
        for record in records {
            let Value::Text(match_doc_no) = &record.match_doc_no else {
                continue;
            };
            if record.invoice_no.is_empty() {
                continue;
            }
            grouped
                .entry(match_doc_no.clone())
                .or_default()
                .push(record.invoice_no.clone());
        }
        let index = Rc::new(grouped);
        CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().as_mut() {
                cache.invoices = Some(Rc::clone(&index));
            }
        });
        index
    }

    /// 按`匹配单据号`汇总收款单统计全部非空`备注`（未去重）；歧义判定同样复用`resolve`。
    pub(crate) fn receipts_index(input_dir: &Path) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if let Some(index) = CACHE.with(|c| c.borrow().as_ref()?.receipts.clone()) {
            return Ok(index);
        }
        let records = receipts::load_records(input_dir)?;
        Ok(CACHE
            .with(|c| c.borrow().as_ref()?.receipts.clone())
            .unwrap_or_else(|| Self::cache_receipts_index(&records)))
    }

    fn cache_receipts_index(records: &[receipts::ReceiptRecord]) -> Rc<MultiValueIndex> {
        let mut grouped: MultiValueIndex = HashMap::new();
        for record in records {
            if record.match_doc_no.is_empty() || record.remark.is_empty() {
                continue;
            }
            grouped
                .entry(record.match_doc_no.clone())
                .or_default()
                .push(record.remark.clone());
        }
        let index = Rc::new(grouped);
        CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().as_mut() {
                cache.receipts = Some(Rc::clone(&index));
            }
        });
        index
    }

    pub(crate) fn print_stats() {
        if let Some(stats) = CACHE.with(|c| c.borrow().as_ref()?.stats) {
            println!(
                "匹配统计：收款单备注 {}，上传参考号 {}，上传发票号 {}，未上传 {}；合计 {}。",
                stats.hits[0],
                stats.hits[1],
                stats.hits[2],
                stats.hits[3],
                stats.hits.iter().sum::<usize>()
            );
            println!(
                "歧义记录：参考号提取 {}，发票号码 {}，收款单备注 {}，上传状态 {}（不同阶段可重复计数）。",
                stats.ambiguous[0], stats.ambiguous[1], stats.ambiguous[2], stats.ambiguous[3]
            );
        }
    }

    pub(crate) fn record_stats(stats: MatchStats) {
        CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().as_mut() {
                cache.stats = Some(stats);
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn stats() -> Option<MatchStats> {
        CACHE.with(|c| c.borrow().as_ref()?.stats)
    }

    pub(crate) fn store_authority(records: &[unionpay::UnionPayRecord]) {
        if CACHE.with(|c| c.borrow().is_some()) {
            Self::cache_authority(records);
        }
    }

    pub(crate) fn store_invoices(records: &[invoice::InvoiceRecord]) {
        if CACHE.with(|c| c.borrow().is_some()) {
            Self::cache_invoice_index(records);
        }
    }

    pub(crate) fn store_receipts(records: &[receipts::ReceiptRecord]) {
        if CACHE.with(|c| c.borrow().is_some()) {
            Self::cache_receipts_index(records);
        }
    }
}

impl Drop for ScopedCache {
    fn drop(&mut self) {
        CACHE.with(|c| *c.borrow_mut() = None);
    }
}

fn is_valid_ref_no(value: &str) -> bool {
    value.len() == 12
        && value.as_bytes()[11] == b'N'
        && value.as_bytes()[..11].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_reference_number_shape() {
        assert!(is_valid_ref_no("16867252734N"));
        assert!(!is_valid_ref_no("16867252734W")); // 结尾不是大写 N
        assert!(!is_valid_ref_no("1686725273N")); // 只有 10 位数字
        assert!(!is_valid_ref_no("16867252734n")); // 小写 n 不算
    }
}
