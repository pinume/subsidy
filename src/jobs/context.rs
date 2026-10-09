use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use super::{Category, MultiValueIndex, invoice, receipts, unionpay};
use crate::model::{ProcessError, Table, Value};

/// 命中分类互斥；不同阶段的歧义可重复计数。
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchStats {
    pub hits: [usize; 4],
    pub ambiguous: [usize; 4],
}

#[derive(Default, Debug)]
pub struct PipelineContext {
    tables: HashMap<Category, Table>,
    authority: Option<Rc<HashSet<String>>>,
    invoices: Option<Rc<MultiValueIndex>>,
    receipts: Option<Rc<MultiValueIndex>>,
    stats: Option<MatchStats>,
}

impl PipelineContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store_table(&mut self, category: Category, table: Table) {
        self.tables.insert(category, table);
    }

    pub fn get_table(&self, category: Category) -> Option<Table> {
        self.tables.get(&category).cloned()
    }

    /// 只读借用缓存中的表并执行闭包，避免整表深拷贝。
    pub fn with_table<F, R>(&self, category: Category, f: F) -> Option<R>
    where
        F: FnOnce(&Table) -> R,
    {
        self.tables.get(&category).map(f)
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
        let index = Self::build_invoice_index(&records);
        self.invoices = Some(Rc::clone(&index));
        Ok(index)
    }

    pub(crate) fn store_invoices(&mut self, records: &[invoice::InvoiceRecord]) {
        let index = Self::build_invoice_index(records);
        self.invoices = Some(index);
    }

    fn build_invoice_index(records: &[invoice::InvoiceRecord]) -> Rc<MultiValueIndex> {
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
        Rc::new(grouped)
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
    fn stores_and_retrieves_table() {
        let mut ctx = PipelineContext::new();
        let table = sample_table();
        assert!(ctx.get_table(Category::Invoice).is_none());
        ctx.store_table(Category::Invoice, table.clone());
        assert_eq!(
            ctx.get_table(Category::Invoice).map(|t| t.rows.len()),
            Some(1)
        );
    }

    #[test]
    fn with_table_borrows_without_cloning() {
        let mut ctx = PipelineContext::new();
        let table = sample_table();
        ctx.store_table(Category::RefundAppliance, table);
        let row_count = ctx.with_table(Category::RefundAppliance, |t| t.rows.len());
        assert_eq!(row_count, Some(1));
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
