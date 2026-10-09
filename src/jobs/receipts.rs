use std::collections::HashMap;
use std::path::Path;

use crate::io::xlsx_reader::{SheetGrid, open_sheets};
use crate::model::{Column, ColumnType, ProcessError, Row, Table, Value};

use super::{
    Category, Job, PipelineContext, build_match_doc_no, cell_text, data_error, parse_date_field,
    text_value,
};

const FILE_NAME: &str = "收款单统计.xlsx";

pub(crate) const SOURCE_HEADERS: [&str; 60] = [
    "日期",
    "销售部门",
    "单据号",
    "联系人",
    "联系电话",
    "销售金额",
    "客户名称",
    "收款日期",
    "销售类别",
    "编号",
    "销售员",
    "预定",
    "服务方式",
    "制单机器",
    "收款机器",
    "制单员",
    "收款员",
    "手工票号",
    "摘要",
    "联营商",
    "商品名称",
    "商品简码",
    "计量单位",
    "库存类型",
    "价格类型",
    "是否负卖",
    "数量",
    "尾款金额",
    "定金金额",
    "实收手续费",
    "实收包装及配件款",
    "其它费用",
    "费用承担供应商",
    "商家贴卡金额",
    "厂家贴卡金额",
    "销售折让金额",
    "已记账",
    "会员卡号",
    "预定单号",
    "预定单日期",
    "原票号",
    "是否作废",
    "作废人",
    "作废日期",
    "兑现使用积分",
    "积分",
    "积分兑现金额",
    "认筹码",
    "手机号",
    "接口同步成功",
    "是否已出库",
    "交易流水号",
    "退货赠品销售金额",
    "认证类型",
    "认证码",
    "已上传",
    "线上支付",
    "记账错误信息",
    "新券码",
    "Crm会员账号",
];

// 源表列位置（1 基）。
const COL_DATE: u32 = 1;
const COL_DOC_NO: u32 = 3;
const COL_SALE_CATEGORY: u32 = 9;
const COL_PRODUCT_NAME: u32 = 21;
const COL_ORIGINAL_TICKET_NO: u32 = 41;

pub(crate) struct ReceiptRecord {
    date: Value,
    doc_no: String,
    product_name: String,
    original_ticket_no: String,
    /// 空字符串表示未生成（日期或单据号为空）。
    pub match_doc_no: String,
    /// 仅用于备注生成，不作为输出字段。
    sale_category: String,
    pub remark: String,
}

pub struct ReceiptsJob;

impl Job for ReceiptsJob {
    fn category(&self) -> Category {
        Category::Receipts
    }

    fn title(&self) -> &'static str {
        "收款单统计"
    }

    fn output_stem(&self) -> &'static str {
        "收款单统计"
    }

    fn run_in_context(
        &self,
        input_dir: &Path,
        ctx: &mut PipelineContext,
    ) -> Result<Table, ProcessError> {
        let records = load_records(input_dir)?;
        ctx.store_receipts(&records);
        let rows = records.into_iter().map(to_row).collect();
        Ok(Table {
            columns: output_columns(),
            rows,
        })
    }
}

/// 读取并计算全部有效明细的退货`备注`，供本任务及第 10 节
/// 销售用券情况统计的`备注`匹配复用；不做输出列裁剪。
pub(crate) fn load_records(input_dir: &Path) -> Result<Vec<ReceiptRecord>, ProcessError> {
    let path = input_dir.join(FILE_NAME);
    if !path.is_file() {
        return Err(ProcessError::NoInput {
            pattern: FILE_NAME.to_string(),
        });
    }

    let sheets = open_sheets(&path)?;
    if sheets.len() != 1 {
        return Err(ProcessError::Structure {
            file: FILE_NAME.to_string(),
            sheet: String::new(),
            detail: format!("工作表数量异常：应为 1 个，实际为 {} 个", sheets.len()),
        });
    }
    let sheet = &sheets[0];
    let sheet_name = sheet.name().to_string();

    let header = sheet.row_texts(2);
    if header.iter().map(String::as_str).collect::<Vec<_>>() != SOURCE_HEADERS {
        return Err(ProcessError::Structure {
            file: FILE_NAME.to_string(),
            sheet: sheet_name,
            detail: "第2行表头与规定的60个字段不一致".to_string(),
        });
    }

    let last_row = sheet.last_value_row().unwrap_or(2);
    let total_marker = sheet.cell(last_row, 1).to_string();
    if total_marker != "合计" {
        return Err(ProcessError::Structure {
            file: FILE_NAME.to_string(),
            sheet: sheet_name,
            detail: format!("最后一个实际有值行第1列应为“合计”，实际为“{total_marker}”"),
        });
    }

    let mut records = Vec::new();
    for row in 3..last_row {
        records.push(read_row(sheet, row, FILE_NAME, &sheet_name)?);
    }

    compute_remarks(&mut records);

    Ok(records)
}

fn read_row(
    sheet: &SheetGrid,
    row: u32,
    file: &str,
    sheet_name: &str,
) -> Result<ReceiptRecord, ProcessError> {
    let date = parse_date_field(&sheet.cell(row, COL_DATE), "日期", file, sheet_name, row)?;

    let text_at = |col: u32, field: &'static str| -> Result<String, ProcessError> {
        let cell = sheet.cell(row, col);
        cell_text(&cell)
            .map_err(|detail| data_error(file, sheet_name, row, field, cell.to_string(), detail))
    };

    let doc_no_value = text_at(COL_DOC_NO, "单据号")?;
    let sale_category = text_at(COL_SALE_CATEGORY, "销售类别")?;
    let product_name = text_at(COL_PRODUCT_NAME, "商品名称")?;
    let original_ticket_no = text_at(COL_ORIGINAL_TICKET_NO, "原票号")?;
    let match_doc_no = build_match_doc_no(&date, &doc_no_value);

    Ok(ReceiptRecord {
        date,
        doc_no: doc_no_value,
        product_name,
        original_ticket_no,
        match_doc_no,
        sale_category,
        remark: String::new(),
    })
}

fn initial_remark(sale_category: &str) -> String {
    match sale_category {
        "退货" => "退货-退单".to_string(),
        _ => String::new(),
    }
}

/// 按非空字段值建立索引：字段值 → 记录下标列表。
fn index_by(
    records: &[ReceiptRecord],
    key_fn: impl Fn(&ReceiptRecord) -> &str,
) -> HashMap<String, Vec<usize>> {
    let mut map: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        let key = key_fn(record);
        if !key.is_empty() {
            map.entry(key.to_string()).or_default().push(index);
        }
    }
    map
}

/// 仅由实际退单启动，沿原票号经过补差/换货单追溯到正常销售原单。
fn compute_remarks(records: &mut [ReceiptRecord]) {
    for record in records.iter_mut() {
        record.remark = initial_remark(&record.sale_category);
    }

    let by_match_doc_no = index_by(records, |r| &r.match_doc_no);
    let mut pending: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.sale_category == "退货")
        .map(|(index, _)| index)
        .collect();
    let mut visited = vec![false; records.len()];
    while let Some(index) = pending.pop() {
        if visited[index] {
            continue;
        }
        visited[index] = true;
        let record = &mut records[index];
        match record.sale_category.as_str() {
            "正常销售" => {
                record.remark = "退货-原单".to_string();
                continue;
            }
            "退货" | "零售补差" | "同型号换货" => {}
            _ => continue,
        }
        if let Some(indices) = by_match_doc_no.get(&record.original_ticket_no) {
            pending.extend(indices);
        }
    }
}

fn output_columns() -> Vec<Column> {
    vec![
        Column {
            name: "日期",
            ty: ColumnType::Date,
        },
        Column {
            name: "单据号",
            ty: ColumnType::Text,
        },
        Column {
            name: "商品名称",
            ty: ColumnType::Text,
        },
        Column {
            name: "原票号",
            ty: ColumnType::Text,
        },
        Column {
            name: "匹配单据号",
            ty: ColumnType::Text,
        },
        Column {
            name: "备注",
            ty: ColumnType::Text,
        },
    ]
}

fn to_row(record: ReceiptRecord) -> Row {
    Row {
        values: vec![
            record.date,
            text_value(record.doc_no),
            text_value(record.product_name),
            text_value(record.original_ticket_no),
            text_value(record.match_doc_no),
            text_value(record.remark),
        ],
        fill: None,
    }
}

#[cfg(test)]
mod tests {
    use rust_xlsxwriter::Workbook;

    use super::*;
    use crate::test_support::unique_temp_path;

    /// 一行明细：日期、单据号、销售类别、商品名称、原票号（按源表列位置写入）。
    struct RowSpec {
        date: &'static str,
        doc_no: &'static str,
        sale_category: &'static str,
        product_name: &'static str,
        original_ticket_no: &'static str,
    }

    fn write_workbook(path: &Path, rows: &[RowSpec]) {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "标题").unwrap();
        for (col, name) in SOURCE_HEADERS.iter().enumerate() {
            sheet.write_string(1, col as u16, *name).unwrap();
        }
        for (index, spec) in rows.iter().enumerate() {
            let row = (2 + index) as u32;
            sheet
                .write_string(row, (COL_DATE - 1) as u16, spec.date)
                .unwrap();
            sheet
                .write_string(row, (COL_DOC_NO - 1) as u16, spec.doc_no)
                .unwrap();
            sheet
                .write_string(row, (COL_SALE_CATEGORY - 1) as u16, spec.sale_category)
                .unwrap();
            sheet
                .write_string(row, (COL_PRODUCT_NAME - 1) as u16, spec.product_name)
                .unwrap();
            sheet
                .write_string(
                    row,
                    (COL_ORIGINAL_TICKET_NO - 1) as u16,
                    spec.original_ticket_no,
                )
                .unwrap();
        }
        let total_row = (2 + rows.len()) as u32;
        sheet.write_string(total_row, 0, "合计").unwrap();
        workbook.save(path).unwrap();
    }

    fn doc_no_of(row: &Row) -> &str {
        match &row.values[1] {
            Value::Text(text) => text.as_str(),
            Value::Empty => "",
            _ => panic!("expected text"),
        }
    }

    fn remark_of(row: &Row) -> &str {
        match &row.values[5] {
            Value::Text(text) => text.as_str(),
            Value::Empty => "",
            _ => panic!("expected text"),
        }
    }

    fn match_doc_no_of(row: &Row) -> &str {
        match &row.values[4] {
            Value::Text(text) => text.as_str(),
            Value::Empty => "",
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn reports_no_input_when_file_missing() {
        let dir = unique_temp_path("receipts-no-input");
        std::fs::create_dir_all(&dir).unwrap();

        let error = ReceiptsJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::NoInput { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_header_mismatch() {
        let dir = unique_temp_path("receipts-bad-header");
        std::fs::create_dir_all(&dir).unwrap();
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "标题").unwrap();
        sheet.write_string(1, 0, "错误表头").unwrap();
        sheet.write_string(2, 0, "合计").unwrap();
        workbook.save(dir.join(FILE_NAME)).unwrap();

        let error = ReceiptsJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::Structure { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_missing_total_row_marker() {
        let dir = unique_temp_path("receipts-bad-total");
        std::fs::create_dir_all(&dir).unwrap();
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.write_string(0, 0, "标题").unwrap();
        for (col, name) in SOURCE_HEADERS.iter().enumerate() {
            sheet.write_string(1, col as u16, *name).unwrap();
        }
        sheet.write_string(2, 0, "不是合计").unwrap();
        workbook.save(dir.join(FILE_NAME)).unwrap();

        let error = ReceiptsJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::Structure { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn generates_match_doc_no_per_example_and_handles_blank_fields() {
        let dir = unique_temp_path("receipts-match-doc-no");
        std::fs::create_dir_all(&dir).unwrap();
        write_workbook(
            &dir.join(FILE_NAME),
            &[
                // project.md 9.4 节示例：2026-01-01 + 收款ZFFX000003 → 260101ZFFX000003。
                RowSpec {
                    date: "2026-01-01",
                    doc_no: "收款ZFFX000003",
                    sale_category: "正常销售",
                    product_name: "商品A",
                    original_ticket_no: "",
                },
                RowSpec {
                    date: "",
                    doc_no: "ZE001",
                    sale_category: "正常销售",
                    product_name: "商品B",
                    original_ticket_no: "",
                },
                RowSpec {
                    date: "2026-01-01",
                    doc_no: "",
                    sale_category: "正常销售",
                    product_name: "商品C",
                    original_ticket_no: "",
                },
            ],
        );

        let table = ReceiptsJob.run(&dir).unwrap();
        assert_eq!(table.columns.len(), 6);
        assert_eq!(table.rows.len(), 3);
        assert_eq!(match_doc_no_of(&table.rows[0]), "260101ZFFX000003");
        assert_eq!(match_doc_no_of(&table.rows[1]), ""); // 日期为空
        assert_eq!(match_doc_no_of(&table.rows[2]), ""); // 单据号为空
        assert_eq!(doc_no_of(&table.rows[0]), "收款ZFFX000003"); // 输出单据号保留“收款”前缀

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_actual_returns_mark_originals_and_preserve_order() {
        let dir = unique_temp_path("receipts-remark-passes");
        std::fs::create_dir_all(&dir).unwrap();

        write_workbook(
            &dir.join(FILE_NAME),
            &[
                // TARGET：仅被补差单关联，没有实际退货，备注保持为空。
                RowSpec {
                    date: "2026-01-10",
                    doc_no: "ZT001",
                    sale_category: "正常销售",
                    product_name: "商品甲",
                    original_ticket_no: "",
                },
                // REBATE：零售补差，即使原票号命中也不生成备注。
                RowSpec {
                    date: "2026-01-11",
                    doc_no: "ZR001",
                    sale_category: "零售补差",
                    product_name: "商品乙",
                    original_ticket_no: "260110ZT001",
                },
                // REBATE_NOMATCH：原票号未命中，备注仍为空。
                RowSpec {
                    date: "2026-01-12",
                    doc_no: "ZR002",
                    sale_category: "零售补差",
                    product_name: "商品丙",
                    original_ticket_no: "NOMATCH001",
                },
                // RETURN：退货，原票号命中 NORMAL_HIT 的匹配单据号，备注固定为“退货-退单”。
                RowSpec {
                    date: "2026-01-13",
                    doc_no: "ZD001",
                    sale_category: "退货",
                    product_name: "商品丁",
                    original_ticket_no: "260114ZN001",
                },
                // NORMAL_HIT：正常销售，匹配单据号命中 RETURN 的原票号 → “退货-原单”。
                RowSpec {
                    date: "2026-01-14",
                    doc_no: "ZN001",
                    sale_category: "正常销售",
                    product_name: "商品戊",
                    original_ticket_no: "",
                },
                // NORMAL_MISS：正常销售，无任何命中，备注保持为空。
                RowSpec {
                    date: "2026-01-15",
                    doc_no: "ZN002",
                    sale_category: "正常销售",
                    product_name: "商品己",
                    original_ticket_no: "",
                },
            ],
        );

        let table = ReceiptsJob.run(&dir).unwrap();
        assert_eq!(table.rows.len(), 6);

        // 顺序必须保持源表相对顺序，不因备注/匹配处理而重排。
        let doc_nos: Vec<&str> = table.rows.iter().map(doc_no_of).collect();
        assert_eq!(
            doc_nos,
            vec!["ZT001", "ZR001", "ZR002", "ZD001", "ZN001", "ZN002"]
        );

        let remarks: Vec<&str> = table.rows.iter().map(remark_of).collect();
        assert_eq!(
            remarks,
            vec![
                "",          // TARGET：仅补差不算退货
                "",          // REBATE：补差单始终留空
                "",          // REBATE_NOMATCH：未命中也留空
                "退货-退单", // RETURN：销售类别精确匹配
                "退货-原单", // NORMAL_HIT：匹配单据号命中 RETURN 的原票号
                "",          // NORMAL_MISS：无命中，保持为空
            ]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn traces_return_chains_through_exchanges_with_missing_keys_and_cycles() {
        let dir = unique_temp_path("receipts-return-chains");
        std::fs::create_dir(&dir).unwrap();
        // 固定日期；各项为单据号、源销售类别、原票号、预期备注。退单先于原单。
        let cases = [
            ("RETURN", "退货", "260110EXCHANGE", "退货-退单"),
            ("EXCHANGE", "同型号换货", "260110REBATE", ""),
            ("REBATE", "零售补差", "260110ORIGINAL", ""),
            ("ORIGINAL", "正常销售", "260110STOP", "退货-原单"),
            ("ORIGINAL", "正常销售", "", "退货-原单"), // 同号多行
            ("BRANCH", "同型号换货", "260110ORIGINAL", ""), // 未退货的分支
            ("STOP", "正常销售", "", ""),              // 命中正常销售后停止
            ("NO_RETURN", "正常销售", "", ""),
            ("ONLY_EXCHANGE", "同型号换货", "260110NO_RETURN", ""),
            ("MISSING", "退货", "NOT_FOUND", "退货-退单"),
            ("EMPTY", "退货", "", "退货-退单"),
            ("CYCLE_RETURN", "退货", "260110CYCLE_A", "退货-退单"),
            ("CYCLE_A", "零售补差", "260110CYCLE_B", ""),
            ("CYCLE_B", "同型号换货", "260110CYCLE_A", ""),
            ("SELF_RETURN", "退货", "260110SELF", "退货-退单"),
            ("SELF", "同型号换货", "260110SELF", ""),
            ("OTHER_RETURN", "退货", "260110OTHER", "退货-退单"),
            ("OTHER", "其他", "260110NO_RETURN", ""),
            ("NOT_EXACT", "退货 ", "260110NO_RETURN", ""),
            ("BLANK_DATE", "正常销售", "", ""),
            ("BLANK_DATE_RETURN", "退货", "BLANK_DATE", "退货-退单"),
            ("BLANK_MATCH_RETURN", "退货", "260110", "退货-退单"),
            ("", "正常销售", "", ""),
            ("SECOND_RETURN", "退货", "260110EXCHANGE", "退货-退单"),
        ];
        let specs: Vec<_> = cases
            .iter()
            .map(|&(doc_no, sale_category, original_ticket_no, _)| RowSpec {
                date: if doc_no == "BLANK_DATE" {
                    ""
                } else {
                    "2026-01-10"
                },
                doc_no,
                sale_category,
                product_name: "商品",
                original_ticket_no,
            })
            .collect();
        let path = dir.join(FILE_NAME);
        write_workbook(&path, &specs);
        let before = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let table = ReceiptsJob.run(&dir).unwrap();
            assert_eq!(table.rows.len(), cases.len());
            for (row, &(doc_no, _, _, expected)) in table.rows.iter().zip(&cases) {
                assert_eq!(doc_no_of(row), doc_no);
                assert_eq!(remark_of(row), expected, "{doc_no}");
            }
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // 再次计算必须重置备注，移除退货来源后不能保留旧标记。
        let mut records = load_records(&dir).unwrap();
        for record in &mut records {
            if record.sale_category == "退货" {
                record.sale_category = "其他".into();
            }
        }
        compute_remarks(&mut records);
        assert!(records.iter().all(|record| record.remark.is_empty()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
