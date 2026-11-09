use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use super::{Category, MultiValueIndex, invoice, receipts, unionpay};
use crate::model::{ProcessError, Table};

/// 命中分类互斥；不同阶段的歧义可重复计数。
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchStats {
    pub hits: [usize; 4],
    pub ambiguous: [usize; 4],
}

impl MatchStats {
    // `hits` 下标：互斥的四类匹配结果。
    pub(crate) const HIT_RECEIPTS: usize = 0;
    pub(crate) const HIT_REFERENCE: usize = 1;
    pub(crate) const HIT_INVOICE: usize = 2;
    pub(crate) const NOT_UPLOADED: usize = 3;
    // `ambiguous` 下标：各阶段歧义记录数。
    pub(crate) const AMB_REFERENCE: usize = 0;
    pub(crate) const AMB_INVOICE_NO: usize = 1;
    pub(crate) const AMB_RECEIPTS: usize = 2;
    pub(crate) const AMB_UPLOAD_STATUS: usize = 3;
}

#[derive(Default, Debug)]
pub struct PipelineContext {
    tables: HashMap<Category, Rc<Table>>,
    authority: Option<Rc<HashSet<String>>>,
    invoices: Option<Rc<MultiValueIndex>>,
    receipts: Option<Rc<MultiValueIndex>>,
    stats: Option<MatchStats>,
}

impl PipelineContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// 已缓存时直接共享该表（不深拷贝），否则运行 `produce` 并缓存结果。
    pub(crate) fn cached_table(
        &mut self,
        category: Category,
        produce: impl FnOnce(&mut Self) -> Result<Table, ProcessError>,
    ) -> Result<Rc<Table>, ProcessError> {
        if let Some(cached) = self.tables.get(&category) {
            return Ok(Rc::clone(cached));
        }
        let table = Rc::new(produce(self)?);
        self.tables.insert(category, Rc::clone(&table));
        Ok(table)
    }

    pub(crate) fn authority(
        &mut self,
        input_dir: &Path,
    ) -> Result<Rc<HashSet<String>>, ProcessError> {
        if let Some(index) = &self.authority {
            return Ok(Rc::clone(index));
        }
        let records = unionpay::load_records(input_dir)?;
        let index = Self::build_authority_index(&records);
        self.authority = Some(Rc::clone(&index));
        Ok(index)
    }

    pub(crate) fn store_authority(&mut self, records: &[unionpay::UnionPayRecord]) {
        let index = Self::build_authority_index(records);
        self.authority = Some(index);
    }

    fn build_authority_index(records: &[unionpay::UnionPayRecord]) -> Rc<HashSet<String>> {
        Rc::new(
            records
                .iter()
                .map(|r| r.retrieval_no.as_str())
                .filter(|v| is_valid_ref_no(v))
                .map(str::to_owned)
                .collect(),
        )
    }

    /// 按`匹配单据号`汇总发票明细全部非空`数电发票号码`（未去重）；歧义判定见`coupons::to_row`
    /// 中复用的`resolve`（与 10.6.2 节"命中权威值需唯一"同一原则，不得任选）。
    pub(crate) fn invoice_index(
        &mut self,
        input_dir: &Path,
    ) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if let Some(index) = &self.invoices {
            return Ok(Rc::clone(index));
        }
        let records = invoice::load_records(input_dir)?;
        let index = Rc::new(invoice::document_index(&records));
        self.invoices = Some(Rc::clone(&index));
        Ok(index)
    }

    pub(crate) fn store_invoices(&mut self, records: &[invoice::InvoiceRecord]) {
        let index = Rc::new(invoice::document_index(records));
        self.invoices = Some(index);
    }

    /// 按`匹配单据号`汇总收款单统计全部非空`备注`（未去重）；歧义判定同样复用`resolve`。
    pub(crate) fn receipts_index(
        &mut self,
        input_dir: &Path,
    ) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if let Some(index) = &self.receipts {
            return Ok(Rc::clone(index));
        }
        let records = receipts::load_records(input_dir)?;
        let index = Self::build_receipts_index(&records);
        self.receipts = Some(Rc::clone(&index));
        Ok(index)
    }

    pub(crate) fn store_receipts(&mut self, records: &[receipts::ReceiptRecord]) {
        let index = Self::build_receipts_index(records);
        self.receipts = Some(index);
    }

    fn build_receipts_index(records: &[receipts::ReceiptRecord]) -> Rc<MultiValueIndex> {
        let mut grouped = MultiValueIndex::new();
        for record in records {
            if record.match_doc_no.is_empty() || record.remark.is_empty() {
                continue;
            }
            grouped.insert(record.match_doc_no.clone(), record.remark.clone());
        }
        Rc::new(grouped)
    }

    pub fn record_stats(&mut self, stats: MatchStats) {
        self.stats = Some(stats);
    }

    #[cfg(test)]
    pub fn stats(&self) -> Option<MatchStats> {
        self.stats
    }

    pub fn print_stats(&self) {
        if let Some(stats) = self.stats {
            println!(
                "匹配统计：收款单备注 {}，上传参考号 {}，上传发票号 {}，未上传 {}；合计 {}。",
                stats.hits[MatchStats::HIT_RECEIPTS],
                stats.hits[MatchStats::HIT_REFERENCE],
                stats.hits[MatchStats::HIT_INVOICE],
                stats.hits[MatchStats::NOT_UPLOADED],
                stats.hits.iter().sum::<usize>()
            );
            println!(
                "歧义记录：参考号提取 {}，发票号码 {}，收款单备注 {}，上传状态 {}（不同阶段可重复计数）。",
                stats.ambiguous[MatchStats::AMB_REFERENCE],
                stats.ambiguous[MatchStats::AMB_INVOICE_NO],
                stats.ambiguous[MatchStats::AMB_RECEIPTS],
                stats.ambiguous[MatchStats::AMB_UPLOAD_STATUS]
            );
        }
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

    fn sample_table() -> Table {
        Table {
            columns: vec![],
            rows: vec![crate::model::Row {
                values: vec![],
                fill: None,
            }],
        }
    }

    #[test]
    fn cached_table_runs_once_and_shares_without_cloning() {
        let mut ctx = PipelineContext::new();
        let first = ctx
            .cached_table(Category::RefundAppliance, |_| Ok(sample_table()))
            .unwrap();
        let second = ctx
            .cached_table(Category::RefundAppliance, |_| unreachable!("已缓存"))
            .unwrap();
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(second.rows.len(), 1);
    }

    #[test]
    fn failed_production_is_not_cached() {
        let mut ctx = PipelineContext::new();
        let failed = ctx.cached_table(Category::Invoice, |_| {
            Err(ProcessError::Read("boom".into()))
        });
        assert!(failed.is_err());
        let retried = ctx.cached_table(Category::Invoice, |_| Ok(sample_table()));
        assert!(retried.is_ok());
    }

    #[test]
    fn records_and_retrieves_match_stats() {
        let mut ctx = PipelineContext::new();
        assert!(ctx.stats().is_none());
        let stats = MatchStats {
            hits: [10, 20, 30, 40],
            ambiguous: [1, 2, 3, 4],
        };
        ctx.record_stats(stats);
        assert_eq!(ctx.stats(), Some(stats));
    }

    #[test]
    fn validates_reference_number_shape() {
        assert!(is_valid_ref_no("16867252734N"));
        assert!(!is_valid_ref_no("16867252734W")); // 结尾不是大写 N
        assert!(!is_valid_ref_no("1686725273N")); // 只有 10 位数字
        assert!(!is_valid_ref_no("16867252734n")); // 小写 n 不算
    }
}
