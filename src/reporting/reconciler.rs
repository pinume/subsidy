use std::collections::HashMap;

use super::data::SheetData;
use super::reader::{UploadColumns, cell_to_string};

/// 商品信息（零拷贝借用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProductInfo<'a> {
    pub category: &'a str,
    pub brand: &'a str,
    pub model: &'a str,
}

/// 对账结果（零拷贝借用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReconciledTransaction<'a> {
    pub product: Option<ProductInfo<'a>>,
    pub status: &'a str,
    pub invoice_no: &'a str,
    pub is_red_flush: bool,
}

impl<'a> ReconciledTransaction<'a> {
    pub fn red_flush_text(&self) -> &'static str {
        if self.is_red_flush { "是" } else { "" }
    }

    pub fn category(&self) -> &'a str {
        self.product.map(|p| p.category).unwrap_or("")
    }

    pub fn brand(&self) -> &'a str {
        self.product.map(|p| p.brand).unwrap_or("")
    }

    pub fn model(&self) -> &'a str {
        self.product.map(|p| p.model).unwrap_or("")
    }
}

#[derive(Debug)]
struct ProductEntry {
    category: String,
    brand: String,
    model: String,
}

#[derive(Debug)]
struct UploadEntry {
    status: String,
    invoice_no: String,
}

#[derive(Debug)]
struct InvoiceEntry {
    invoice_type: String,
    invoice_status: String,
    match_doc_no: String,
}

#[derive(Debug)]
pub(crate) struct TransactionReconciler {
    product_map: HashMap<String, ProductEntry>,
    upload_map: HashMap<String, UploadEntry>,
    invoice_map: HashMap<String, InvoiceEntry>,
}

impl TransactionReconciler {
    /// 校验必要表头并构建哈希索引；缺列时立即 Fail-Fast 报错。
    pub fn from_inputs(
        sales: &SheetData,
        app_upload: &SheetData,
        dig_upload: &SheetData,
        invoices: &SheetData,
    ) -> Result<Self, String> {
        // 1. Upfront header validation across all 4 input sheets (fail-fast)
        let sales_h = &sales.header;
        let sales_doc_idx = sales_h.require("匹配单据号", "销售用券情况统计.xlsx")?;
        let sales_cat_idx = sales_h.require("财务大类", "销售用券情况统计.xlsx")?;
        let sales_brand_idx = sales_h.require("品牌", "销售用券情况统计.xlsx")?;
        let sales_name_idx = sales_h.require("商品名称", "销售用券情况统计.xlsx")?;

        let app_cols = UploadColumns::from_header(&app_upload.header, "已上传家电电脑.xlsx")?;
        let dig_cols = UploadColumns::from_header(&dig_upload.header, "已上传数码.xlsx")?;

        let inv_h = &invoices.header;
        let inv_no_idx = inv_h
            .find(&["数电发票号码"])
            .ok_or("发票明细缺少数电发票号码")?;
        let inv_type_idx = inv_h.find(&["开票类型"]).ok_or("发票明细缺少开票类型")?;
        let inv_st_idx = inv_h.find(&["开票状态"]).ok_or("发票明细缺少开票状态")?;
        let inv_doc_idx = inv_h.require("匹配单据号", "发票明细.xlsx")?;

        // 2. Index the sales record by its document number (匹配单据号)
        let mut product_map = HashMap::new();
        for row in &sales[1..] {
            let document = cell_to_string(&row[sales_doc_idx]);
            if !document.is_empty() {
                product_map.insert(
                    document,
                    ProductEntry {
                        category: cell_to_string(&row[sales_cat_idx]),
                        brand: cell_to_string(&row[sales_brand_idx]),
                        model: cell_to_string(&row[sales_name_idx]),
                    },
                );
            }
        }

        // 3. Build index for uploaded records (appliance + digital)
        let mut upload_map = HashMap::new();
        for (rows, cols) in [(app_upload, app_cols), (dig_upload, dig_cols)] {
            for row in &rows[1..] {
                let reference_no = cell_to_string(&row[cols.reference]);
                if !reference_no.is_empty() {
                    upload_map.insert(
                        reference_no,
                        UploadEntry {
                            status: cell_to_string(&row[cols.status]),
                            invoice_no: cell_to_string(&row[cols.invoice]),
                        },
                    );
                }
            }
        }

        // 4. Build index for invoice records
        let mut invoice_map = HashMap::new();
        for row in &invoices[1..] {
            let invoice_no = cell_to_string(&row[inv_no_idx]);
            if !invoice_no.is_empty() {
                invoice_map.insert(
                    invoice_no,
                    InvoiceEntry {
                        invoice_type: cell_to_string(&row[inv_type_idx]),
                        invoice_status: cell_to_string(&row[inv_st_idx]),
                        match_doc_no: cell_to_string(&row[inv_doc_idx]),
                    },
                );
            }
        }

        Ok(Self {
            product_map,
            upload_map,
            invoice_map,
        })
    }

    /// 根据银联检索号与门店备注执行对账与商品关联。
    ///
    /// 业务规则引用：
    /// - `docs/reporting-rules.md` 第 5.1.1 节规则 2：门店备注为“已退货”时，状态固定为“已退货”。
    /// - `docs/reporting-rules.md` 第 5.1.1 节规则 3：未命中已上传记录时，状态填“未提交”。
    /// - `docs/reporting-rules.md` 第 5.1.1 节规则 4：发票开票类型为“红票”或开票状态为“已红冲”时，标记为“是”，否则留空。
    /// - `docs/reporting-rules.md` 第 5.1.1 节规则 5：通过已上传发票号关联发票明细匹配单据号，再命中销售用券商品信息（品类、品牌、型号）。
    pub fn reconcile<'a>(
        &'a self,
        reference_no: &str,
        store_remark: &str,
    ) -> ReconciledTransaction<'a> {
        let upload_entry = self.upload_map.get(reference_no);
        let invoice_no = match upload_entry {
            Some(entry) if !entry.invoice_no.is_empty() => entry.invoice_no.as_str(),
            _ => "",
        };

        let invoice_entry = self.invoice_map.get(invoice_no);
        let product = invoice_entry
            .and_then(|entry| self.product_map.get(&entry.match_doc_no))
            .map(|entry| ProductInfo {
                category: entry.category.as_str(),
                brand: entry.brand.as_str(),
                model: entry.model.as_str(),
            });

        let status = if store_remark == "已退货" {
            "已退货"
        } else if let Some(entry) = upload_entry {
            entry.status.as_str()
        } else {
            "未提交"
        };

        let is_red_flush = if invoice_no.is_empty() {
            false
        } else if let Some(entry) = invoice_entry {
            entry.invoice_type == "红票" || entry.invoice_status == "已红冲"
        } else {
            false
        };

        ReconciledTransaction {
            product,
            status,
            invoice_no,
            is_red_flush,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_reconciler() -> TransactionReconciler {
        TransactionReconciler {
            product_map: HashMap::new(),
            upload_map: HashMap::new(),
            invoice_map: HashMap::new(),
        }
    }

    #[test]
    fn reconciles_product_through_document_chain() {
        let mut reconciler = empty_reconciler();

        // upload: ref-1 -> status "待审核", invoice "inv-101"
        reconciler.upload_map.insert(
            "ref-1".to_string(),
            UploadEntry {
                status: "待审核".to_string(),
                invoice_no: "inv-101".to_string(),
            },
        );

        // invoice: inv-101 -> type "蓝票", status "开票完成", doc "doc-999"
        reconciler.invoice_map.insert(
            "inv-101".to_string(),
            InvoiceEntry {
                invoice_type: "蓝票".to_string(),
                invoice_status: "开票完成".to_string(),
                match_doc_no: "doc-999".to_string(),
            },
        );

        // product: doc-999 -> ("冰箱", "海尔", "BCD-500")
        reconciler.product_map.insert(
            "doc-999".to_string(),
            ProductEntry {
                category: "冰箱".to_string(),
                brand: "海尔".to_string(),
                model: "BCD-500".to_string(),
            },
        );

        let result = reconciler.reconcile("ref-1", "");

        assert_eq!(result.status, "待审核");
        assert_eq!(result.invoice_no, "inv-101");
        assert!(!result.is_red_flush);
        assert_eq!(result.red_flush_text(), "");
        assert_eq!(result.category(), "冰箱");
        assert_eq!(result.brand(), "海尔");
        assert_eq!(result.model(), "BCD-500");
        assert_eq!(
            result.product,
            Some(ProductInfo {
                category: "冰箱",
                brand: "海尔",
                model: "BCD-500",
            })
        );
    }

    #[test]
    fn returns_overridden_status_when_store_remark_is_returned() {
        let mut reconciler = empty_reconciler();
        reconciler.upload_map.insert(
            "ref-ret".to_string(),
            UploadEntry {
                status: "审核通过未回款".to_string(),
                invoice_no: "inv-ret".to_string(),
            },
        );

        // Even though upload status is "审核通过未回款", store remark "已退货" must take priority
        let result = reconciler.reconcile("ref-ret", "已退货");
        assert_eq!(result.status, "已退货");
        assert_eq!(result.invoice_no, "inv-ret");
    }

    #[test]
    fn defaults_to_unsubmitted_when_unmatched() {
        let reconciler = empty_reconciler();
        let result = reconciler.reconcile("ref-unsubmitted", "");
        assert_eq!(result.status, "未提交");
        assert_eq!(result.invoice_no, "");
        assert_eq!(result.product, None);
        assert_eq!(result.category(), "");
        assert_eq!(result.brand(), "");
        assert_eq!(result.model(), "");
        assert!(!result.is_red_flush);
        assert_eq!(result.red_flush_text(), "");
    }

    #[test]
    fn flags_red_ticket_or_flushed_invoice_as_red_flush() {
        let mut reconciler = empty_reconciler();

        // Red invoice type
        reconciler.upload_map.insert(
            "ref-red".to_string(),
            UploadEntry {
                status: "待审核".to_string(),
                invoice_no: "inv-red".to_string(),
            },
        );
        reconciler.invoice_map.insert(
            "inv-red".to_string(),
            InvoiceEntry {
                invoice_type: "红票".to_string(),
                invoice_status: "开票完成".to_string(),
                match_doc_no: "doc-1".to_string(),
            },
        );

        // Flushed status
        reconciler.upload_map.insert(
            "ref-flushed".to_string(),
            UploadEntry {
                status: "待审核".to_string(),
                invoice_no: "inv-flushed".to_string(),
            },
        );
        reconciler.invoice_map.insert(
            "inv-flushed".to_string(),
            InvoiceEntry {
                invoice_type: "蓝票".to_string(),
                invoice_status: "已红冲".to_string(),
                match_doc_no: "doc-2".to_string(),
            },
        );

        // Normal blue
        reconciler.upload_map.insert(
            "ref-blue".to_string(),
            UploadEntry {
                status: "待审核".to_string(),
                invoice_no: "inv-blue".to_string(),
            },
        );
        reconciler.invoice_map.insert(
            "inv-blue".to_string(),
            InvoiceEntry {
                invoice_type: "蓝票".to_string(),
                invoice_status: "开票完成".to_string(),
                match_doc_no: "doc-3".to_string(),
            },
        );

        assert!(reconciler.reconcile("ref-red", "").is_red_flush);
        assert_eq!(reconciler.reconcile("ref-red", "").red_flush_text(), "是");

        assert!(reconciler.reconcile("ref-flushed", "").is_red_flush);
        assert_eq!(
            reconciler.reconcile("ref-flushed", "").red_flush_text(),
            "是"
        );

        assert!(!reconciler.reconcile("ref-blue", "").is_red_flush);
        assert_eq!(reconciler.reconcile("ref-blue", "").red_flush_text(), "");
    }

    #[test]
    fn from_inputs_fails_when_sales_missing_required_column() {
        use calamine::Data;
        let sales = SheetData::new(vec![vec![
            Data::String("商品名称".to_string()),
            Data::String("品牌".to_string()),
        ]]);
        let app_upload = SheetData::new(vec![vec![
            Data::String("检索参考号".to_string()),
            Data::String("状态".to_string()),
            Data::String("发票号码".to_string()),
            Data::String("补贴金额".to_string()),
        ]]);
        let dig_upload = SheetData::new(vec![vec![
            Data::String("检索参考号".to_string()),
            Data::String("状态".to_string()),
            Data::String("发票号码".to_string()),
            Data::String("补贴金额".to_string()),
        ]]);
        let invoices = SheetData::new(vec![vec![
            Data::String("数电发票号码".to_string()),
            Data::String("开票类型".to_string()),
            Data::String("开票状态".to_string()),
            Data::String("匹配单据号".to_string()),
        ]]);

        let err = TransactionReconciler::from_inputs(&sales, &app_upload, &dig_upload, &invoices)
            .unwrap_err();
        assert!(err.contains("销售用券情况统计.xlsx: 缺少必要列 [匹配单据号]"));
    }

    #[test]
    fn from_inputs_builds_indexes_and_reconciles() {
        use calamine::Data;
        let sales = SheetData::new(vec![
            vec![
                Data::String("匹配单据号".to_string()),
                Data::String("财务大类".to_string()),
                Data::String("品牌".to_string()),
                Data::String("商品名称".to_string()),
            ],
            vec![
                Data::String("doc-100".to_string()),
                Data::String("彩电".to_string()),
                Data::String("创维".to_string()),
                Data::String("75A3D 4K超高清".to_string()),
            ],
        ]);
        let app_upload = SheetData::new(vec![
            vec![
                Data::String("检索参考号".to_string()),
                Data::String("状态".to_string()),
                Data::String("发票号码".to_string()),
                Data::String("补贴金额".to_string()),
            ],
            vec![
                Data::String("ref-999".to_string()),
                Data::String("审核通过未回款".to_string()),
                Data::String("inv-888".to_string()),
                Data::String("200".to_string()),
            ],
        ]);
        let dig_upload = SheetData::new(vec![vec![
            Data::String("检索参考号".to_string()),
            Data::String("状态".to_string()),
            Data::String("发票号码".to_string()),
            Data::String("补贴金额".to_string()),
        ]]);
        let invoices = SheetData::new(vec![
            vec![
                Data::String("数电发票号码".to_string()),
                Data::String("开票类型".to_string()),
                Data::String("开票状态".to_string()),
                Data::String("匹配单据号".to_string()),
            ],
            vec![
                Data::String("inv-888".to_string()),
                Data::String("蓝票".to_string()),
                Data::String("开票完成".to_string()),
                Data::String("doc-100".to_string()),
            ],
        ]);

        let reconciler =
            TransactionReconciler::from_inputs(&sales, &app_upload, &dig_upload, &invoices)
                .unwrap();
        let res = reconciler.reconcile("ref-999", "");
        assert_eq!(res.status, "审核通过未回款");
        assert_eq!(res.invoice_no, "inv-888");
        assert!(!res.is_red_flush);
        assert_eq!(res.category(), "彩电");
        assert_eq!(res.brand(), "创维");
        assert_eq!(res.model(), "75A3D 4K超高清");
        assert_eq!(
            res.product,
            Some(ProductInfo {
                category: "彩电",
                brand: "创维",
                model: "75A3D 4K超高清",
            })
        );
    }
}
