use std::collections::{HashMap, HashSet};

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

#[derive(Debug, Default, PartialEq, Eq)]
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
    product: ProductEntry,
}

#[derive(Debug, Default)]
pub(crate) struct TransactionReconciler {
    // None marks a conflicting reference and remains ambiguous for later candidates.
    sales_map: HashMap<String, Option<ProductEntry>>,
    sales_documents: HashMap<String, Option<String>>,
    blue_invoices: HashMap<String, String>,
    upload_map: HashMap<String, UploadEntry>,
    invoice_map: HashMap<String, InvoiceEntry>,
}

impl TransactionReconciler {
    /// 校验必要表头并构建哈希索引；缺列时立即 Fail-Fast 报错。
    pub fn from_inputs(
        app_upload: &SheetData,
        dig_upload: &SheetData,
        invoices: &SheetData,
        sales: &SheetData,
    ) -> Result<Self, String> {
        let app_cols = UploadColumns::from_header(&app_upload.header, "已上传家电电脑.xlsx")?;
        let dig_cols = UploadColumns::from_header(&dig_upload.header, "已上传数码.xlsx")?;

        let inv_h = &invoices.header;
        let inv_no_idx = inv_h
            .find(&["数电发票号码"])
            .ok_or("发票明细缺少数电发票号码")?;
        let inv_type_idx = inv_h.find(&["开票类型"]).ok_or("发票明细缺少开票类型")?;
        let inv_st_idx = inv_h.find(&["开票状态"]).ok_or("发票明细缺少开票状态")?;
        let inv_category_idx = inv_h.require("大类", "发票明细.xlsx")?;
        let inv_brand_idx = inv_h.require("品牌", "发票明细.xlsx")?;
        let inv_product_name_idx = inv_h.require("主要商品名称", "发票明细.xlsx")?;

        let inv_doc_idx = inv_h.require("匹配单据号", "发票明细.xlsx")?;

        // Build index for uploaded records (appliance + digital).
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

        // Index invoices and their parsed product fields by invoice number.
        let mut invoice_map = HashMap::new();
        let mut blue_candidates: HashMap<String, (HashSet<String>, HashSet<String>)> =
            HashMap::new();
        for row in &invoices[1..] {
            let invoice_no = cell_to_string(&row[inv_no_idx]);
            if !invoice_no.is_empty() {
                let document = row
                    .get(inv_doc_idx)
                    .map(ToString::to_string)
                    .unwrap_or_default();
                if !document.is_empty() && cell_to_string(&row[inv_type_idx]) == "蓝票" {
                    let (all, completed) = blue_candidates.entry(document).or_default();
                    all.insert(invoice_no.clone());
                    if cell_to_string(&row[inv_st_idx]) == "开票完成" {
                        completed.insert(invoice_no.clone());
                    }
                }
                invoice_map.insert(
                    invoice_no,
                    InvoiceEntry {
                        invoice_type: cell_to_string(&row[inv_type_idx]),
                        invoice_status: cell_to_string(&row[inv_st_idx]),
                        product: ProductEntry {
                            category: cell_to_string(&row[inv_category_idx]),
                            brand: cell_to_string(&row[inv_brand_idx]),
                            model: cell_to_string(&row[inv_product_name_idx]),
                        },
                    },
                );
            }
        }

        let sales_reference_idx = sales.header.require("参考号", "销售用券情况统计.xlsx")?;
        let sales_category_idx = sales.header.require("财务大类", "销售用券情况统计.xlsx")?;
        let sales_brand_idx = sales.header.require("品牌", "销售用券情况统计.xlsx")?;
        let sales_name_idx = sales.header.require("商品名称", "销售用券情况统计.xlsx")?;
        let sales_doc_idx = sales
            .header
            .require("匹配单据号", "销售用券情况统计.xlsx")?;
        let mut sales_map = HashMap::new();
        let mut sales_documents = HashMap::new();
        for row in &sales[1..] {
            let reference_no = row[sales_reference_idx].to_string();
            if reference_no.is_empty() {
                continue;
            }
            let document = row
                .get(sales_doc_idx)
                .map(ToString::to_string)
                .unwrap_or_default();
            if !document.is_empty() {
                sales_documents
                    .entry(reference_no.clone())
                    .and_modify(|candidate: &mut Option<String>| {
                        if candidate.as_ref() != Some(&document) {
                            *candidate = None;
                        }
                    })
                    .or_insert(Some(document));
            }
            let product = ProductEntry {
                category: row[sales_category_idx].to_string(),
                brand: row[sales_brand_idx].to_string(),
                model: row[sales_name_idx].to_string(),
            };
            sales_map
                .entry(reference_no)
                .and_modify(|candidate: &mut Option<ProductEntry>| {
                    if candidate.as_ref() != Some(&product) {
                        *candidate = None;
                    }
                })
                .or_insert(Some(product));
        }

        // Completed blue invoices take priority; ambiguity never falls back to an arbitrary blue.
        let blue_invoices = blue_candidates
            .into_iter()
            .filter_map(|(document, (all, completed))| {
                let candidates = if completed.is_empty() { all } else { completed };
                if candidates.len() == 1 {
                    Some((document, candidates.into_iter().next().unwrap()))
                } else {
                    None
                }
            })
            .collect();

        Ok(Self {
            sales_map,
            sales_documents,
            blue_invoices,
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
    /// - `docs/reporting-rules.md` 第 5.1.1 节规则 5：通过已上传发票号关联发票明细，读取发票清洗时生成的商品信息。
    ///
    /// 发票号按第 5.1.1 节规则 4 优先取上传号码，空时通过唯一销售单据查找蓝票。
    /// 商品信息按第 5.1.1 节规则 2 优先使用完整且唯一的销售组合，否则整组回退到发票。
    pub fn reconcile<'a>(
        &'a self,
        reference_no: &str,
        store_remark: &str,
    ) -> ReconciledTransaction<'a> {
        let upload_entry = self.upload_map.get(reference_no);
        let invoice_no = match upload_entry {
            Some(entry) if !entry.invoice_no.is_empty() => entry.invoice_no.as_str(),
            _ => self
                .sales_documents
                .get(reference_no)
                .and_then(Option::as_ref)
                .and_then(|document| self.blue_invoices.get(document))
                .map(String::as_str)
                .unwrap_or(""),
        };

        let invoice_entry = self.invoice_map.get(invoice_no);
        let product = self
            .sales_map
            .get(reference_no)
            .and_then(Option::as_ref)
            .filter(|entry| {
                !entry.category.is_empty() && !entry.brand.is_empty() && !entry.model.is_empty()
            })
            .or_else(|| invoice_entry.map(|entry| &entry.product))
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

        let is_red_flush = !invoice_no.is_empty()
            && invoice_entry
                .is_some_and(|e| e.invoice_type == "红票" || e.invoice_status == "已红冲");

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

    fn table(rows: Vec<Vec<&str>>) -> SheetData {
        use calamine::Data;
        SheetData::new(
            rows.into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|value| Data::String(value.into()))
                        .collect()
                })
                .collect(),
        )
    }

    fn empty_sales() -> SheetData {
        sales_table(&[])
    }

    fn sales_table(rows: &[&[&str]]) -> SheetData {
        use calamine::Data;
        SheetData::new(
            std::iter::once(&["参考号", "财务大类", "品牌", "商品名称", "匹配单据号"][..])
                .chain(rows.iter().copied())
                .map(|row| {
                    row.iter()
                        .map(|value| Data::String((*value).into()))
                        .collect()
                })
                .collect(),
        )
    }

    #[test]
    fn reconciles_product_through_invoice_number() {
        let mut reconciler = TransactionReconciler::default();

        // upload: ref-1 -> status "待审核", invoice "inv-101"
        reconciler.upload_map.insert(
            "ref-1".to_string(),
            UploadEntry {
                status: "待审核".to_string(),
                invoice_no: "inv-101".to_string(),
            },
        );

        // invoice: inv-101 -> type "蓝票", status "开票完成"
        // product: inv-101 -> ("冰箱", "海尔", "海尔-冰箱-BCD-500")
        reconciler.invoice_map.insert(
            "inv-101".to_string(),
            InvoiceEntry {
                invoice_type: "蓝票".to_string(),
                invoice_status: "开票完成".to_string(),
                product: ProductEntry {
                    category: "冰箱".to_string(),
                    brand: "海尔".to_string(),
                    model: "海尔-冰箱-BCD-500".to_string(),
                },
            },
        );

        let result = reconciler.reconcile("ref-1", "");

        assert_eq!(result.status, "待审核");
        assert_eq!(result.invoice_no, "inv-101");
        assert!(!result.is_red_flush);
        assert_eq!(result.red_flush_text(), "");
        assert_eq!(result.category(), "冰箱");
        assert_eq!(result.brand(), "海尔");
        assert_eq!(result.model(), "海尔-冰箱-BCD-500");
        assert_eq!(
            result.product,
            Some(ProductInfo {
                category: "冰箱",
                brand: "海尔",
                model: "海尔-冰箱-BCD-500",
            })
        );
    }

    #[test]
    fn returns_overridden_status_when_store_remark_is_returned() {
        let mut reconciler = TransactionReconciler::default();
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
        let reconciler = TransactionReconciler::default();
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
        let mut reconciler = TransactionReconciler::default();

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
                product: ProductEntry::default(),
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
                product: ProductEntry::default(),
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
                product: ProductEntry::default(),
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
    fn sales_products_are_unique_complete_and_fall_back_as_a_group() {
        use calamine::Data;
        let keys = [
            "complete",
            "same",
            "conflict",
            "empty-category",
            "empty-brand",
            "empty-name",
            "missing-sale",
            "partial-invoice",
            "missing-invoice",
        ];
        let mut upload_rows = vec![vec!["检索参考号", "状态", "发票号码", "补贴金额"]];
        for key in keys {
            let invoice = match key {
                "partial-invoice" => "partial",
                "missing-invoice" => "absent",
                _ => "invoice",
            };
            upload_rows.push(vec![key, "审核终止", invoice, ""]);
        }
        let uploads = table(upload_rows);
        let digital = table(vec![vec!["检索参考号", "状态", "发票号码", "补贴金额"]]);
        let invoices = table(vec![
            vec![
                "数电发票号码",
                "开票类型",
                "开票状态",
                "大类",
                "品牌",
                "主要商品名称",
                "匹配单据号",
            ],
            vec![
                "invoice",
                "红票",
                "开票完成",
                "冰箱",
                "海尔",
                "海尔-冰箱-BCD-500",
            ],
            vec!["partial", "蓝票", "开票完成", "彩电", "", "发票完整名称"],
        ]);
        let sales = sales_table(&[
            &["complete", "洗衣机", "美的", "小天鹅-洗衣机-TG12TP3"],
            &["same", "洗衣机", "美的", "小天鹅-洗衣机-TG12TP3"],
            &["same", "洗衣机", "美的", "小天鹅-洗衣机-TG12TP3"],
            &["conflict", "洗衣机", "美的", "型号A"],
            &["conflict", "冰箱", "海尔", "型号B"],
            // Later repetition must not turn an ambiguous key back into a unique one.
            &["conflict", "洗衣机", "美的", "型号A"],
            &["empty-category", "", "美的", "销售名称"],
            &["empty-brand", "洗衣机", "", "销售名称"],
            &["empty-name", "洗衣机", "美的", ""],
            &["partial-invoice", "洗衣机", "", "销售名称"],
            &["missing-invoice", "洗衣机", "", "销售名称"],
            &["unsubmitted", "数码", "OPPO", "OPPO手机"],
            &["padded", "洗衣机", "美的", " 小天鹅-洗衣机-TG12TP3 "],
            &[" spaced-reference ", "数码", "OPPO", "OPPO手机"],
            &["", "数码", "OPPO", "空参考号不能匹配"],
        ]);
        let reconciler =
            TransactionReconciler::from_inputs(&uploads, &digital, &invoices, &sales).unwrap();
        for key in ["complete", "same"] {
            let result = reconciler.reconcile(key, "");
            assert_eq!(
                (result.category(), result.brand(), result.model()),
                ("洗衣机", "美的", "小天鹅-洗衣机-TG12TP3")
            );
            assert_eq!(result.status, "审核终止");
            assert_eq!(result.invoice_no, "invoice");
            assert!(result.is_red_flush);
        }
        for key in [
            "conflict",
            "empty-category",
            "empty-brand",
            "empty-name",
            "missing-sale",
        ] {
            let result = reconciler.reconcile(key, "");
            assert_eq!(
                (result.category(), result.brand(), result.model()),
                ("冰箱", "海尔", "海尔-冰箱-BCD-500"),
                "{key}"
            );
        }
        let partial = reconciler.reconcile("partial-invoice", "");
        assert_eq!(
            (partial.category(), partial.brand(), partial.model()),
            ("彩电", "", "发票完整名称")
        );
        assert!(
            reconciler
                .reconcile("missing-invoice", "")
                .product
                .is_none()
        );
        assert!(reconciler.reconcile("", "").product.is_none());
        let unsubmitted = reconciler.reconcile("unsubmitted", "");
        assert_eq!(unsubmitted.status, "未提交");
        assert_eq!(unsubmitted.invoice_no, "");
        assert_eq!(unsubmitted.model(), "OPPO手机");
        assert_eq!(
            reconciler.reconcile("padded", "").model(),
            " 小天鹅-洗衣机-TG12TP3 "
        );
        assert!(
            reconciler
                .reconcile("spaced-reference", "")
                .product
                .is_none()
        );
        let returned = reconciler.reconcile("complete", "已退货");
        assert_eq!(returned.status, "已退货");
        assert_eq!(returned.brand(), "美的");
        assert_eq!(returned.invoice_no, "invoice");
        assert!(returned.is_red_flush);

        for missing in 0..5 {
            let mut invalid_sales = empty_sales();
            invalid_sales.header = super::super::reader::HeaderMap::from_header_row(
                &invalid_sales[0]
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index == missing {
                            Data::String("缺失列".into())
                        } else {
                            value.clone()
                        }
                    })
                    .collect::<Vec<_>>(),
            );
            let error =
                TransactionReconciler::from_inputs(&uploads, &digital, &invoices, &invalid_sales)
                    .unwrap_err();
            assert!(error.contains("销售用券情况统计.xlsx: 缺少必要列"));
        }
    }

    #[test]
    fn empty_upload_invoice_uses_unique_sales_document_and_preferred_blue_invoice() {
        let uploads = table(vec![
            vec!["检索参考号", "状态", "发票号码", "补贴金额"],
            vec!["primary", "待审核", "red-only", ""],
            vec!["primary-missing", "审核失败", "unknown-invoice", ""],
            vec!["empty-upload", "审核终止", "", ""],
        ]);
        let digital = table(vec![vec!["检索参考号", "状态", "发票号码", "补贴金额"]]);
        let invoice_rows = [
            ["done", "done-no", "蓝票", "开票完成"],
            ["flushed", "flushed-no", "蓝票", "已红冲"],
            ["failed", "failed-no", "蓝票", "开票失败"],
            ["red", "red-only", "红票", "开票完成"],
            ["mixed", "old-no", "蓝票", "已红冲"],
            ["mixed", "new-no", "蓝票", "开票完成"],
            ["mixed", "red-mixed", "红票", "开票完成"],
            ["many-done", "done-a", "蓝票", "开票完成"],
            ["many-done", "done-b", "蓝票", "开票完成"],
            ["many-done", "failed-c", "蓝票", "开票失败"],
            ["many-unfinished", "unfinished-a", "蓝票", "已红冲"],
            ["many-unfinished", "unfinished-b", "蓝票", "开票失败"],
            ["duplicate", "duplicate-no", "蓝票", "开票完成"],
            ["duplicate", "duplicate-no", "蓝票", "开票完成"],
            ["blank-number", "", "蓝票", "开票完成"],
            ["", "blank-document-no", "蓝票", "开票完成"],
            ["two-doc-a", "shared-no", "蓝票", "开票完成"],
            ["two-doc-b", "shared-no", "蓝票", "开票完成"],
            ["blue-red", "blue-no", "蓝票", "开票失败"],
            ["blue-red", "red-no", "红票", "开票完成"],
        ];
        let mut rows = vec![vec![
            "匹配单据号",
            "数电发票号码",
            "开票类型",
            "开票状态",
            "大类",
            "品牌",
            "主要商品名称",
        ]];
        for [document, number, kind, status] in invoice_rows {
            rows.push(vec![
                document,
                number,
                kind,
                status,
                "冰箱",
                "海尔",
                "发票完整商品名称",
            ]);
        }
        let invoices = table(rows);
        let sales = sales_table(&[
            &["primary", "", "", "", "done"],
            &["primary-missing", "", "", "", "done"],
            &["empty-upload", "", "", "", "done"],
            &["done", "", "", "", "done"],
            &["done", "", "", "", ""],
            &["done", "", "", "", "done"],
            &["flushed", "", "", "", "flushed"],
            &["failed", "", "", "", "failed"],
            &["red", "", "", "", "red"],
            &["mixed", "", "", "", "mixed"],
            &["many-done", "", "", "", "many-done"],
            &["many-unfinished", "", "", "", "many-unfinished"],
            &["duplicate", "", "", "", "duplicate"],
            &["blank-number", "", "", "", "blank-number"],
            &["blank-document", "", "", "", ""],
            &["absent-document", "", "", "", "absent-document"],
            &["two-docs", "", "", "", "two-doc-a"],
            &["two-docs", "", "", "", "two-doc-b"],
            &["two-docs", "", "", "", "two-doc-a"],
            &["blue-red", "", "", "", "blue-red"],
            // Product ambiguity does not prevent a unique document from resolving the invoice.
            &["product-conflict", "数码", "OPPO", "商品A", "done"],
            &["product-conflict", "冰箱", "海尔", "商品B", "done"],
            &[
                "sales-complete",
                "洗衣机",
                "美的",
                "销售完整商品名称",
                "flushed",
            ],
            &["", "", "", "", "done"],
        ]);
        let reconciler =
            TransactionReconciler::from_inputs(&uploads, &digital, &invoices, &sales).unwrap();
        for (reference, expected) in [
            ("done", "done-no"),
            ("flushed", "flushed-no"),
            ("failed", "failed-no"),
            ("mixed", "new-no"),
            ("duplicate", "duplicate-no"),
            ("blue-red", "blue-no"),
            ("product-conflict", "done-no"),
        ] {
            let result = reconciler.reconcile(reference, "");
            assert_eq!(result.invoice_no, expected, "{reference}");
            assert_eq!(result.status, "未提交");
            assert_eq!(
                (result.category(), result.brand(), result.model()),
                ("冰箱", "海尔", "发票完整商品名称")
            );
            assert_eq!(result.is_red_flush, reference == "flushed");
        }
        for reference in [
            "red",
            "many-done",
            "many-unfinished",
            "blank-number",
            "blank-document",
            "absent-document",
            "two-docs",
            "absent-sale",
            "",
        ] {
            let result = reconciler.reconcile(reference, "");
            assert_eq!(result.invoice_no, "", "{reference}");
            assert!(result.product.is_none());
            assert!(!result.is_red_flush);
        }
        let primary = reconciler.reconcile("primary", "");
        assert_eq!(primary.invoice_no, "red-only");
        assert_eq!(primary.status, "待审核");
        assert!(primary.is_red_flush);
        let primary_missing = reconciler.reconcile("primary-missing", "");
        assert_eq!(primary_missing.invoice_no, "unknown-invoice");
        assert_eq!(primary_missing.status, "审核失败");
        assert!(primary_missing.product.is_none());
        let empty_upload = reconciler.reconcile("empty-upload", "");
        assert_eq!(empty_upload.invoice_no, "done-no");
        assert_eq!(empty_upload.status, "审核终止");
        let returned = reconciler.reconcile("flushed", "已退货");
        assert_eq!(returned.invoice_no, "flushed-no");
        assert_eq!(returned.status, "已退货");
        assert!(returned.is_red_flush);
        let complete = reconciler.reconcile("sales-complete", "");
        assert_eq!(complete.invoice_no, "flushed-no");
        assert_eq!(complete.model(), "销售完整商品名称");
        assert!(complete.is_red_flush);

        let invalid_invoices = table(vec![vec![
            "数电发票号码",
            "开票类型",
            "开票状态",
            "大类",
            "品牌",
            "主要商品名称",
        ]]);
        let error =
            TransactionReconciler::from_inputs(&uploads, &digital, &invalid_invoices, &sales)
                .unwrap_err();
        assert!(error.contains("发票明细.xlsx: 缺少必要列 [匹配单据号]"));
    }

    #[test]
    fn from_inputs_fails_when_invoice_missing_required_column() {
        use calamine::Data;
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

        let err =
            TransactionReconciler::from_inputs(&app_upload, &dig_upload, &invoices, &empty_sales())
                .unwrap_err();
        assert!(err.contains("发票明细.xlsx: 缺少必要列 [大类]"));
    }

    #[test]
    fn from_inputs_builds_indexes_and_reconciles() {
        use calamine::Data;
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
                Data::String("大类".to_string()),
                Data::String("品牌".to_string()),
                Data::String("主要商品名称".to_string()),
                Data::String("匹配单据号".to_string()),
            ],
            vec![
                Data::String("inv-888".to_string()),
                Data::String("蓝票".to_string()),
                Data::String("开票完成".to_string()),
                Data::String("彩电".to_string()),
                Data::String("创维".to_string()),
                Data::String("创维-彩电-75A3D 4K超高清".to_string()),
            ],
        ]);

        let reconciler =
            TransactionReconciler::from_inputs(&app_upload, &dig_upload, &invoices, &empty_sales())
                .unwrap();
        let res = reconciler.reconcile("ref-999", "");
        assert_eq!(res.status, "审核通过未回款");
        assert_eq!(res.invoice_no, "inv-888");
        assert!(!res.is_red_flush);
        assert_eq!(res.category(), "彩电");
        assert_eq!(res.brand(), "创维");
        assert_eq!(res.model(), "创维-彩电-75A3D 4K超高清");
        assert_eq!(
            res.product,
            Some(ProductInfo {
                category: "彩电",
                brand: "创维",
                model: "创维-彩电-75A3D 4K超高清",
            })
        );
    }
}
