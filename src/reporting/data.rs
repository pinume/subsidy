use super::reader::{HeaderMap, extract_unique_merchant_code, read_sheet_rows};
use calamine::Data;
use std::ops::Deref;
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) enum CellKind {
    Text,
    CenteredText,
    Date,
    DateTime,
    Money,
    Percent,
}

impl CellKind {
    fn for_field(field: &str) -> Self {
        match field {
            "交易日期" | "开票日期" => Self::Date,
            "清算时间" | "交易时间" | "交易完成时间" | "开票时间" | "提交时间" | "更新时间"
            | "签收时间" => Self::DateTime,
            "销售金额" | "实收销售金额" | "补贴金额" | "发票金额" | "交易金额" | "清算金额"
            | "手续费" | "T0手续费" | "D1手续费" | "优惠金额" | "分期手续费" | "subsideAmt" => {
                Self::Money
            }
            "补贴比例" => Self::Percent,
            "终端号"
            | "交易类型"
            | "模版类型"
            | "地区编码"
            | "是否属于 AI 产品"
            | "ocrModify"
            | "modifyStatus"
            | "introduceInvoiceFlag"
            | "是否交旧"
            | "是否自提"
            | "收货地址是否农村地区" => Self::CenteredText,
            // 其他支付 is intentionally rendered as text in refund reports.
            _ => Self::Text,
        }
    }
}

pub(crate) struct ReportColumn {
    source: Option<usize>,
    pub kind: CellKind,
}

impl ReportColumn {
    pub fn new(source: Option<usize>, field: &str) -> Self {
        Self {
            source,
            kind: CellKind::for_field(field),
        }
    }

    pub fn cell<'a>(&self, row: &'a [Data]) -> &'a Data {
        self.source
            .and_then(|col| row.get(col))
            .unwrap_or(&Data::Empty)
    }
}

pub(crate) struct SheetData {
    rows: Vec<Vec<Data>>,
    pub header: HeaderMap,
}

impl SheetData {
    pub fn select<'a>(
        &self,
        fields: impl IntoIterator<Item = &'a str>,
        label: &str,
    ) -> Result<Vec<ReportColumn>, String> {
        fields
            .into_iter()
            .map(|field| {
                let aliases: &[&str] = match field {
                    "S/N码" => &["S/N码", "sn码"],
                    _ => &[field],
                };
                let source = self
                    .header
                    .find(aliases)
                    .ok_or_else(|| format!("{label}: 缺少必要列 [{field}]"))?;
                Ok(ReportColumn::new(Some(source), field))
            })
            .collect()
    }

    pub fn new(rows: Vec<Vec<Data>>) -> Self {
        let header = HeaderMap::from_header_row(&rows[0]);
        Self { rows, header }
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        Ok(Self::new(read_sheet_rows(path)?))
    }
}

impl Deref for SheetData {
    type Target = [Vec<Data>];
    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

pub(crate) struct CommonInputs {
    pub sales: SheetData,
    pub app_upload: SheetData,
    pub dig_upload: SheetData,
    pub invoices: SheetData,
    pub app_refund: SheetData,
    pub dig_refund: SheetData,
    pub app_store_code: String,
    pub dig_store_code: String,
}

impl CommonInputs {
    pub fn load(input: &Path) -> Result<Self, String> {
        let sales = SheetData::load(&input.join("销售用券情况统计.xlsx"))?;
        let app_upload = SheetData::load(&input.join("已上传家电电脑.xlsx"))?;
        let dig_upload = SheetData::load(&input.join("已上传数码.xlsx"))?;
        let invoices = SheetData::load(&input.join("发票明细.xlsx"))?;
        let app_refund = SheetData::load(&input.join("回款明细家电电脑.xlsx"))?;
        let dig_refund = SheetData::load(&input.join("回款明细数码.xlsx"))?;
        let app_store_code =
            extract_unique_merchant_code(&app_upload, &app_upload.header, "已上传家电电脑.xlsx")?;
        let dig_store_code =
            extract_unique_merchant_code(&dig_upload, &dig_upload.header, "已上传数码.xlsx")?;
        Ok(Self {
            sales,
            app_upload,
            dig_upload,
            invoices,
            app_refund,
            dig_refund,
            app_store_code,
            dig_store_code,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_path;
    use std::cell::OnceCell;

    #[test]
    fn shared_inputs_are_loaded_once_and_preserve_upload_statuses() {
        let root = unique_temp_path("shared-reports");
        std::fs::create_dir(&root).unwrap();
        for name in [
            "销售用券情况统计.xlsx",
            "已上传家电电脑.xlsx",
            "已上传数码.xlsx",
            "发票明细.xlsx",
            "回款明细家电电脑.xlsx",
            "回款明细数码.xlsx",
        ] {
            let mut book = rust_xlsxwriter::Workbook::new();
            let sheet = book.add_worksheet();
            sheet.write_string(0, 0, "商户号").unwrap();
            sheet.write_string(0, 1, "状态").unwrap();
            sheet.write_string(1, 0, "001").unwrap();
            sheet.write_string(1, 1, "审核终止").unwrap();
            book.save(root.join(name)).unwrap();
        }
        let cache = OnceCell::new();
        let inputs = cache
            .get_or_init(|| CommonInputs::load(&root))
            .as_ref()
            .unwrap();
        assert_eq!(inputs.app_store_code, "001");
        assert_eq!(inputs.app_upload[1][1], "审核终止");
        assert_eq!(
            read_sheet_rows(&root.join("已上传家电电脑.xlsx")).unwrap()[1][1],
            "审核终止"
        );
        std::fs::remove_dir_all(&root).unwrap();
        let second = cache
            .get_or_init(|| panic!("common inputs were read twice"))
            .as_ref()
            .unwrap();
        assert!(std::ptr::eq(inputs, second));
    }
}
