use super::reader::{
    HeaderMap, cell_to_decimal, cell_to_string, extract_unique_merchant_code, read_sheet_rows,
};
use calamine::{Data, DataType};
use rust_decimal::Decimal;
use std::collections::HashSet;
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

    /// 清洗层保留重复导出：同一非空键只保留`更新时间`最新的数据行（重新提交后以最新为准），
    /// 最新时间相同的行都保留，由调用方按各自规则处理；没有更新时间视为最旧，键为空的行全部保留。
    /// 按源顺序返回。
    pub fn latest_rows(&self, key_idx: usize, updated_idx: usize) -> Vec<&Vec<Data>> {
        fn cell(row: &[Data], idx: usize) -> &Data {
            row.get(idx).unwrap_or(&Data::Empty)
        }
        crate::utils::latest::latest_by_key(
            &self[1..],
            |row| Some(cell_to_string(cell(row, key_idx))).filter(|key| !key.is_empty()),
            |row| cell(row, updated_idx).as_datetime(),
        )
    }
}

/// 已上传明细的内部视图：按表头名称读取字段，并拥有汇总与财务统筹的两种去重口径。
/// 各操作在调用时校验所需列，缺列错误带文件名。
#[derive(Clone, Copy)]
pub(crate) struct UploadSheet<'a> {
    rows: &'a SheetData,
    label: &'a str,
}

/// 已上传明细常用字段的列位置；只能由 `UploadSheet::fields` 校验后取得。
#[derive(Clone, Copy)]
pub(crate) struct UploadFields {
    status: usize,
    subsidy: usize,
    invoice: usize,
}

impl UploadFields {
    pub fn status(&self, row: &[Data]) -> String {
        cell_to_string(&row[self.status])
    }

    pub fn subsidy(&self, row: &[Data]) -> Decimal {
        cell_to_decimal(&row[self.subsidy]).unwrap_or(Decimal::ZERO)
    }

    pub fn invoice(&self, row: &[Data]) -> String {
        cell_to_string(&row[self.invoice])
    }
}

impl<'a> UploadSheet<'a> {
    pub fn new(rows: &'a SheetData, label: &'a str) -> Self {
        Self { rows, label }
    }

    /// 校验 `状态`、`补贴金额`、`检索参考号`、`发票号码`。
    pub fn fields(&self) -> Result<UploadFields, String> {
        let h = &self.rows.header;
        let status = h.require("状态", self.label)?;
        let subsidy = h.require("补贴金额", self.label)?;
        h.require("检索参考号", self.label)?;
        let invoice = h.require("发票号码", self.label)?;
        Ok(UploadFields {
            status,
            subsidy,
            invoice,
        })
    }

    /// 报表规则第4.1.1节：金额和笔数不能因重复导出翻倍。同一`实时清分UUID`按`SheetData::latest_rows`
    /// 只取更新时间最新的记录；最新时间相同时每个状态各留一条。UUID 为空无法判断重复，全部保留。
    /// 返回保留的行（源顺序）与去掉的行数。不用 UniqueIndex：它把状态不同的重复判为冲突并全部丢弃。
    pub fn current(&self) -> Result<(Vec<&'a Vec<Data>>, usize), String> {
        let rows = self.rows;
        let uuid_idx = rows.header.require("实时清分UUID", self.label)?;
        let updated_idx = rows.header.require("更新时间", self.label)?;
        let status_idx = rows.header.require("状态", self.label)?;
        let cell =
            |row: &'a Vec<Data>, idx: usize| cell_to_string(row.get(idx).unwrap_or(&Data::Empty));
        let mut seen = HashSet::new();
        let current: Vec<_> = rows
            .latest_rows(uuid_idx, updated_idx)
            .into_iter()
            .filter(|row| {
                let uuid = cell(row, uuid_idx);
                uuid.is_empty() || seen.insert((uuid, cell(row, status_idx)))
            })
            .collect();
        let duplicates = rows.len() - 1 - current.len();
        Ok((current, duplicates))
    }

    /// 报表规则第3.2节第5条：同一参考号只取`更新时间`最新的候选（与清洗第10.12.2节一致）；
    /// 最新时间并列的候选全部返回，由调用方判定冲突。空参考号不返回。
    pub fn latest_by_reference(&self) -> Result<Vec<(String, &'a Vec<Data>)>, String> {
        let reference_idx = self.rows.header.require("检索参考号", self.label)?;
        let updated_idx = self.rows.header.require("更新时间", self.label)?;
        Ok(self
            .rows
            .latest_rows(reference_idx, updated_idx)
            .into_iter()
            .map(|row| (cell_to_string(&row[reference_idx]), row))
            .filter(|(reference, _)| !reference.is_empty())
            .collect())
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
