use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

pub(crate) use super::context::MatchStats;
use super::context::PipelineContext;
use super::{Category, MultiValueIndex, invoice, receipts, unionpay};
use crate::model::{ProcessError, Table};

thread_local! {
    static CACHE: RefCell<Option<PipelineContext>> = const { RefCell::new(None) };
}

pub(crate) struct ScopedCache;

#[allow(dead_code)]
impl ScopedCache {
    pub(crate) fn activate() -> Self {
        CACHE.with(|c| *c.borrow_mut() = Some(PipelineContext::new()));
        ScopedCache
    }

    pub(crate) fn is_active() -> bool {
        CACHE.with(|c| c.borrow().is_some())
    }

    pub(crate) fn with_context_mut<F, R>(f: F) -> R
    where
        F: FnOnce(&mut PipelineContext) -> R,
    {
        CACHE.with(|c| {
            let mut borrow = c.borrow_mut();
            let ctx = borrow.as_mut().expect("ScopedCache 未激活");
            f(ctx)
        })
    }

    pub(crate) fn get(category: Category) -> Option<Table> {
        CACHE.with(|c| c.borrow().as_ref()?.get_table(category))
    }

    /// 只读借用缓存中的表并执行闭包，避免整表深拷贝。
    pub(crate) fn with_table<F, R>(category: Category, f: F) -> Option<R>
    where
        F: FnOnce(&Table) -> R,
    {
        CACHE.with(|c| {
            let borrow = c.borrow();
            borrow.as_ref()?.with_table(category, f)
        })
    }

    pub(crate) fn put(category: Category, table: &Table) {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.store_table(category, table.clone());
            }
        });
    }

    pub(crate) fn authority(input_dir: &Path) -> Result<Rc<HashSet<String>>, ProcessError> {
        if Self::is_active() {
            Self::with_context_mut(|ctx| ctx.authority(input_dir))
        } else {
            let mut ctx = PipelineContext::new();
            ctx.authority(input_dir)
        }
    }

    /// 按`匹配单据号`汇总发票明细全部非空`数电发票号码`（未去重）；歧义判定见`coupons::to_row`
    /// 中复用的`resolve`（与 10.6.2 节"命中权威值需唯一"同一原则，不得任选）。
    pub(crate) fn invoice_index(input_dir: &Path) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if Self::is_active() {
            Self::with_context_mut(|ctx| ctx.invoice_index(input_dir))
        } else {
            let mut ctx = PipelineContext::new();
            ctx.invoice_index(input_dir)
        }
    }

    /// 按`匹配单据号`汇总收款单统计全部非空`备注`（未去重）；歧义判定同样复用`resolve`。
    pub(crate) fn receipts_index(input_dir: &Path) -> Result<Rc<MultiValueIndex>, ProcessError> {
        if Self::is_active() {
            Self::with_context_mut(|ctx| ctx.receipts_index(input_dir))
        } else {
            let mut ctx = PipelineContext::new();
            ctx.receipts_index(input_dir)
        }
    }

    pub(crate) fn print_stats() {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow().as_ref() {
                ctx.print_stats();
            }
        });
    }

    pub(crate) fn record_stats(stats: MatchStats) {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.record_stats(stats);
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn stats() -> Option<MatchStats> {
        CACHE.with(|c| c.borrow().as_ref()?.stats())
    }

    pub(crate) fn store_authority(records: &[unionpay::UnionPayRecord]) {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.store_authority(records);
            }
        });
    }

    pub(crate) fn store_invoices(records: &[invoice::InvoiceRecord]) {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.store_invoices(records);
            }
        });
    }

    pub(crate) fn store_receipts(records: &[receipts::ReceiptRecord]) {
        CACHE.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                ctx.store_receipts(records);
            }
        });
    }
}

impl Drop for ScopedCache {
    fn drop(&mut self) {
        CACHE.with(|c| *c.borrow_mut() = None);
    }
}
