//! 家电、数码台账（§6）：直接读取原始补贴明细，按商户筛选、按拨付批次分组后输出 26 列。
use std::collections::HashMap;
use std::path::Path;

use rust_xlsxwriter::Workbook;

use super::excel::{ReportError, ReportSheetBuilder};
use super::styles::StylePool;
use crate::io::xlsx_reader::RawCell;
use crate::jobs::refund::{
    self, APPLIANCE_SYNONYMS, OUTPUT_FIELDS, REFUND_APPLIANCE, REFUND_DIGITAL,
};
use crate::jobs::{self, MERCHANT_APPLIANCE, MERCHANT_DIGITAL, invoice};
use crate::model::{ProcessError, Value};
use crate::utils::product::normalize_sales_brand;

struct Ledger {
    title: &'static str,
    sheet: &'static str,
    merchant: &'static str,
    merchant_field: &'static str,
    fields: [&'static str; 26],
    /// 输出字段的同义源字段；`None` 表示只读同名字段。
    synonyms: fn(&str) -> Option<&'static [&'static str]>,
    /// 与发票`数电发票号码`关联的台账字段（§6.1）。
    invoice_field: &'static str,
    /// 家电：发票金额、购买方名称先读源表，空值按发票补充（§6.2.2）；
    /// 数码：购买方名称只取发票（§6.3.2）。
    fill_invoice_details: bool,
}

const APPLIANCE: Ledger = Ledger {
    title: "2026年家电以旧换新活动商家交易数据台账（线下）",
    sheet: "家电台账",
    merchant: MERCHANT_APPLIANCE,
    merchant_field: "核销商编",
    fields: [
        "拨付批次",
        "发票状态",
        "品牌品类",
        "序号",
        "创建时间",
        "交易完成时间",
        "交易参考号",
        "商户订单号",
        "交易订单号",
        "销售企业名称",
        "核销商编",
        "其他支付",
        "销售金额",
        "实收销售金额",
        "补贴金额",
        "补贴比例",
        "SN码",
        "所在地区",
        "商品编码",
        "能耗等级",
        "编码品类",
        "商品名称",
        "发票金额",
        "发票号",
        "发票头/购买方名称",
        "ID",
    ],
    // 复用家电回款清洗的同义字段及歧义检查。
    synonyms: |field| {
        OUTPUT_FIELDS
            .iter()
            .position(|name| *name == field)
            .map(|index| APPLIANCE_SYNONYMS[index])
    },
    invoice_field: "发票号",
    fill_invoice_details: true,
};

const DIGITAL: Ledger = Ledger {
    title: "2026年数码以旧换新活动商家交易数据台账（线下）",
    sheet: "数码台账",
    merchant: MERCHANT_DIGITAL,
    merchant_field: "商户编号",
    fields: [
        "拨付批次",
        "发票状态",
        "发票头/购买方名称",
        "序号",
        "提交时间",
        "商户订单号",
        "参考号",
        "销售企业名称",
        "门店名称/县域编码",
        "商户编号",
        "核销日期",
        "商品编码",
        "销售金额",
        "补贴金额",
        "补贴比例/补贴限额",
        "SN码",
        "原始SN码",
        "券号/数据唯一标识",
        "所在地区",
        "类别",
        "商品名称",
        "发票号码",
        "物流单号",
        "备注",
        "ID",
        "客户购买品类",
    ],
    // 只接受 §6.3.1 列出的旧源字段，避免过渡期批次误取其他列。
    synonyms: |field| {
        Some(match field {
            "销售企业名称" => &["销售企业名称", "销方名称"],
            "商户编号" => &["商户编号", "经销商编号"],
            "销售金额" => &["销售金额", "应收销售金额"],
            "补贴金额" => &["补贴金额", "补贴销售金额"],
            "类别" => &["类别", "品类"],
            "商品名称" => &["商品名称", "商品明细"],
            "券号/数据唯一标识" => &["券号/数据唯一标识", "券号"],
            _ => return None,
        })
    },
    invoice_field: "发票号码",
    fill_invoice_details: false,
};

pub(crate) fn generate_appliance(input_dir: &Path, output: &Path) -> Result<Vec<String>, String> {
    generate(&APPLIANCE, &REFUND_APPLIANCE, input_dir, output)
}

pub(crate) fn generate_digital(input_dir: &Path, output: &Path) -> Result<Vec<String>, String> {
    generate(&DIGITAL, &REFUND_DIGITAL, input_dir, output)
}

fn generate(
    ledger: &Ledger,
    source: &refund::RefundJob,
    input_dir: &Path,
    output: &Path,
) -> Result<Vec<String>, String> {
    let mut rows = read_rows(ledger, source, input_dir).map_err(|e| e.to_string())?;
    if let Some(target) = ledger.fields.iter().position(|f| *f == "品牌品类") {
        let [name, category] = ["商品名称", "编码品类"].map(|f| position(ledger, f));
        for row in &mut rows {
            row[target] = brand_category(text_of(&row[name]), text_of(&row[category]))
                .map_or(Value::Empty, Value::Text);
        }
    }
    fill_from_invoices(ledger, &mut rows, input_dir)?;
    write(ledger, &rows, output)?;
    Ok(Vec::new())
}

fn read_rows(
    ledger: &Ledger,
    source: &refund::RefundJob,
    input_dir: &Path,
) -> Result<Vec<Vec<Value>>, ProcessError> {
    let merchant_index = position(ledger, ledger.merchant_field);
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for sheet in source.batch_sheets(input_dir)? {
        let structure = |detail: String| sheet.structure_error(detail);
        let find = |names: &[&str]| jobs::resolve_synonym_column(sheet.header(), names, 0);
        let mut columns = Vec::with_capacity(ledger.fields.len());
        for field in &ledger.fields {
            let names = (ledger.synonyms)(field).unwrap_or(std::slice::from_ref(field));
            let mut column = find(names).map_err(structure)?;
            if column.is_none() && *field == "交易完成时间" {
                column = find(&["交易时间"]).map_err(structure)?;
            }
            columns.push(column);
        }
        let merchant_column = columns[merchant_index]
            .ok_or_else(|| structure(format!("缺少商户筛选字段 {}", ledger.merchant_field)))?;

        for row in sheet.data_rows() {
            if !sheet.merchant_matches(row, merchant_column, ledger.merchant) {
                continue;
            }
            let values = ledger
                .fields
                .iter()
                .zip(&columns)
                .map(|(field, column)| match (*field, column) {
                    // 由发票补充，不读取源表同名列。
                    ("发票状态" | "品牌品类", _) => Value::Empty,
                    ("发票头/购买方名称", _) if !ledger.fill_invoice_details => {
                        Value::Empty
                    }
                    (_, Some(col)) => cell_value(field, &sheet.cell(row, *col)),
                    (_, None) => Value::Empty,
                })
                .collect();
            rows.push(values);
        }
    }
    // §6.1：按拨付批次首次出现的顺序稳定分组，批次内保持源顺序；序号读取源表并随记录保留。
    let batch = position(ledger, "拨付批次");
    let mut first_seen = HashMap::new();
    for row in &rows {
        let next = first_seen.len();
        first_seen
            .entry(text_of(&row[batch]).trim().to_string())
            .or_insert(next);
    }
    rows.sort_by_key(|row| first_seen[text_of(&row[batch]).trim()]);
    Ok(rows)
}

fn position(ledger: &Ledger, field: &str) -> usize {
    ledger
        .fields
        .iter()
        .position(|name| *name == field)
        .unwrap()
}

/// 无法按字段类型解析的值保留原文，不丢弃（如其他支付`-`、补贴限额文本）。
fn cell_value(field: &str, cell: &RawCell) -> Value {
    let original = || Value::Text(cell.to_string());
    match field {
        "创建时间" | "交易完成时间" | "提交时间" => jobs::cell_datetime_or_text(cell),
        "核销日期" => match jobs::cell_datetime_or_text(cell) {
            Value::DateTime(datetime) => Value::Date(datetime.date()),
            other => other,
        },
        "其他支付" | "销售金额" | "实收销售金额" | "补贴金额" | "发票金额" => {
            jobs::cell_amount(cell).map_or_else(|_| original(), jobs::amount_value)
        }
        "补贴比例" | "补贴比例/补贴限额" => refund::cell_ratio(cell).map_or_else(
            |_| original(),
            |ratio| ratio.map_or(Value::Empty, Value::Ratio),
        ),
        _ => jobs::text_value(jobs::cell_text(cell).unwrap_or_else(|_| cell.to_string())),
    }
}

fn text_of(value: &Value) -> &str {
    match value {
        Value::Text(text) => text,
        _ => "",
    }
}

/// §6.2.3 台账识别的家电品牌写法（真实数据中出现的品牌）。
const APPLIANCE_BRANDS: [&str; 23] = [
    "海尔",
    "卡萨帝",
    "统帅",
    "Leader",
    "美的",
    "小天鹅",
    "COLMO",
    "华凌",
    "东芝",
    "海信",
    "Vidda",
    "格力",
    "TCL",
    "创维",
    "美菱",
    "奥克斯",
    "A.O.史密斯",
    "西门子",
    "博世",
    "万家乐",
    "方太",
    "小鸭",
    "科龙",
];

/// §6.2.3 名称中无品牌时，仅按这些完整型号补充品牌。
const MODEL_BRANDS: [(&str, &str); 5] = [
    ("TNDD20-08AIDE", "小天鹅"),
    ("65A6Q", "海信"),
    ("85A6Q", "海信"),
    ("KFR-50GW/CX2S", "COLMO"),
    ("VL5R-PRO", "Vidda"),
];

/// 文本首或尾唯一出现的品牌写法（忽略大小写）；未出现或多个品牌时为 `None`。
fn edge_brand(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let mut found = APPLIANCE_BRANDS.into_iter().filter(|brand| {
        let brand = brand.to_lowercase();
        lower.starts_with(&brand) || lower.ends_with(&brand)
    });
    match (found.next(), found.next()) {
        (Some(brand), None) => Some(brand),
        _ => None,
    }
}

/// §6.2.3：从商品名称与编码品类共同提取品牌和品类。编码品类为空时留空；品牌依次取自
/// 商品名称、编码品类首尾及完整型号，归并后拼接为`品牌品类`；无唯一品牌时只填品类。
fn brand_category(name: &str, code_category: &str) -> Option<String> {
    let code_category = code_category.trim();
    if code_category.is_empty() {
        return None;
    }
    let mut category = code_category
        .split_once('-')
        .map_or(code_category, |(_, category)| category.trim());
    // 去掉发票式`*品类*`前缀，与发票商品名称清洗一致。
    let name = invoice::clean_product_name(name.trim());
    let name = name.trim();
    let brand = if let Some(brand) = edge_brand(name) {
        Some(brand)
    } else if let Some(brand) = edge_brand(category) {
        // 品类文本自带品牌时去掉，避免拼接后重复。
        let len = brand.len(); // 品牌写法为 ASCII 或汉字，小写不改变字节长度。
        category = if category.to_lowercase().starts_with(&brand.to_lowercase()) {
            category[len..].trim()
        } else {
            category[..category.len() - len].trim()
        };
        Some(brand)
    } else {
        MODEL_BRANDS
            .iter()
            .find(|(model, _)| *model == name)
            .map(|(_, brand)| *brand)
    };
    Some(match brand {
        Some(brand) => format!("{}{category}", ledger_brand(brand)),
        None => category.to_string(),
    })
}

/// 台账专用归并写法，其余沿用销售用券财务归并；不改变其他报表口径。
fn ledger_brand(brand: &str) -> String {
    match brand {
        "A.O.史密斯" => "AO史密斯".to_string(),
        "Leader" => "海尔".to_string(),
        "Vidda" => "海信".to_string(),
        "东芝" => "美的".to_string(),
        other => normalize_sales_brand(other),
    }
}

fn is_blank(value: &Value) -> bool {
    match value {
        Value::Empty => true,
        Value::Text(text) => text.trim().is_empty(),
        _ => false,
    }
}

/// §6.1：开票状态为`已红冲`时填写发票状态；购买方名称、家电发票金额只补空值（§6.2.2、§6.3.2）。
fn fill_from_invoices(
    ledger: &Ledger,
    rows: &mut [Vec<Value>],
    input_dir: &Path,
) -> Result<(), String> {
    let records = invoice::load_records(input_dir).map_err(|e| e.to_string())?;
    let mut by_number = HashMap::new();
    for record in &records {
        let number = record.invoice_no.trim();
        if !number.is_empty() && by_number.insert(number, record).is_some() {
            return Err(format!("发票数电发票号码重复，无法唯一补充台账：{number}"));
        }
    }
    let number = position(ledger, ledger.invoice_field);
    let status = position(ledger, "发票状态");
    for row in rows {
        let Value::Text(key) = &row[number] else {
            continue;
        };
        let Some(record) = by_number.get(key.trim()) else {
            continue;
        };
        if record.status().trim() == "已红冲" {
            row[status] = Value::Text("已红冲".to_string());
        }
        let buyer = position(ledger, "发票头/购买方名称");
        if is_blank(&row[buyer]) {
            row[buyer] = jobs::text_value(record.buyer_name().to_string());
        }
        if ledger.fill_invoice_details {
            let amount = position(ledger, "发票金额");
            if is_blank(&row[amount]) {
                row[amount] = record.total_amount().clone();
            }
        }
    }
    Ok(())
}

fn write(ledger: &Ledger, rows: &[Vec<Value>], output: &Path) -> Result<(), ReportError> {
    let styles = StylePool::default();
    let mut workbook = Workbook::new();
    let mut sheet = ReportSheetBuilder::new(&mut workbook, ledger.sheet, &styles)?;
    let last_col = ledger.fields.len() as u16 - 1;
    sheet.ws.set_column_range_width(0, last_col, 18.0)?;
    sheet.ws.set_row_height(0, 30.0)?;
    sheet
        .ws
        .merge_range(0, 0, 0, last_col, ledger.title, &styles.title)?;
    sheet.cur_row = 1;
    sheet.write_headers(&ledger.fields)?;
    sheet.set_freeze_panes(2, 0)?;
    sheet.ws.autofilter(1, 0, 1 + rows.len() as u32, last_col)?;
    for (index, values) in rows.iter().enumerate() {
        let row = 2 + index as u32;
        for (col, value) in values.iter().enumerate() {
            let col = col as u16;
            let ws = &mut *sheet.ws;
            match value {
                Value::Empty => ws.write_blank(row, col, &styles.text_left)?,
                Value::Text(text) => {
                    ws.write_string_with_format(row, col, text, &styles.text_left)?
                }
                Value::Decimal(amount) => ws.write_with_format(row, col, *amount, &styles.money)?,
                Value::Integer(number) => {
                    ws.write_with_format(row, col, *number, &styles.int_count)?
                }
                Value::Ratio(ratio) => ws.write_with_format(row, col, *ratio, &styles.percent)?,
                Value::Date(date) => ws.write_with_format(row, col, date, &styles.date)?,
                Value::DateTime(datetime) => {
                    ws.write_with_format(row, col, datetime, &styles.datetime)?
                }
            };
        }
    }
    workbook.save(output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use calamine::{Data, DataType, Reader};

    use super::*;
    use crate::test_support::unique_temp_path;

    type Sheet<'a> = (&'a str, &'a [&'a str], Vec<Vec<String>>);

    fn write_book(path: &Path, header_row: u32, sheets: &[Sheet]) {
        let mut book = Workbook::new();
        for (name, header, rows) in sheets {
            let ws = book.add_worksheet();
            ws.set_name(*name).unwrap();
            for (col, field) in header.iter().enumerate() {
                ws.write_string(header_row, col as u16, *field).unwrap();
            }
            for (index, row) in rows.iter().enumerate() {
                for (col, value) in row.iter().enumerate() {
                    ws.write_string(header_row + 1 + index as u32, col as u16, value)
                        .unwrap();
                }
            }
        }
        book.save(path).unwrap();
    }

    fn row(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// 发票源表：第6行为30列表头，只填写数电发票号码、购方名称、合计含税金额与开票状态。
    fn write_invoices(dir: &Path, name: &str, records: &[(&str, &str, &str, &str)]) {
        let rows = records
            .iter()
            .map(|(number, buyer, total, status)| {
                let mut values = vec![String::new(); invoice::SOURCE_HEADERS.len()];
                values[8] = number.to_string();
                values[9] = buyer.to_string();
                values[16] = total.to_string();
                values[27] = status.to_string();
                values
            })
            .collect();
        write_book(
            &dir.join(name),
            5,
            &[("发票", &invoice::SOURCE_HEADERS, rows)],
        );
    }

    fn read_output(path: &Path) -> (String, Vec<Vec<Data>>) {
        let mut book: calamine::Xlsx<_> = calamine::open_workbook(path).unwrap();
        let name = book.sheet_names()[0].clone();
        assert_eq!(book.sheet_names().len(), 1);
        // 标题合并 A1:Z1（26列）。
        let merges = book.merge_cells_by_sheet_name(&name).unwrap();
        assert_eq!(
            merges.iter().map(|d| (d.start, d.end)).collect::<Vec<_>>(),
            [((0, 0), (0, 25))]
        );
        let range = book.worksheet_range(&name).unwrap();
        (name, range.rows().map(<[Data]>::to_vec).collect())
    }

    fn text(cell: &Data) -> String {
        match cell {
            Data::String(text) => text.clone(),
            Data::Empty => String::new(),
            other => panic!("expected text, got {other:?}"),
        }
    }

    const OLD_APPLIANCE: [&str; 12] = [
        "序号",
        "拨付批次",
        "交易时间",
        "商户订单号",
        "销方名称",
        " 商户编号 ",
        "应收销售金额",
        "补贴金额",
        "发票号",
        "交易参考号",
        "编码品类",
        "商品名称",
    ];
    const NEW_APPLIANCE: [&str; 12] = [
        "序号",
        "拨付批次",
        "创建时间",
        "交易时间",
        "交易完成时间",
        "商户订单号",
        "核销商编",
        "销售金额",
        "补贴金额",
        "发票金额",
        "发票号",
        "发票头/购买方名称",
    ];

    #[test]
    fn appliance_ledger_filters_maps_headers_and_fills_invoices_without_overwriting() {
        let dir = unique_temp_path("ledger-appliance");
        std::fs::create_dir_all(&dir).unwrap();
        let m = MERCHANT_APPLIANCE;
        write_book(
            &dir.join("2026年家电类以旧换新补贴明细已经回款下发.xlsx"),
            0,
            &[
                (
                    "第一批",
                    &OLD_APPLIANCE,
                    vec![
                        row(&[
                            "1-03",
                            "B1",
                            "2026-01-01 10:00:00",
                            "O1",
                            "企业甲",
                            m,
                            "100",
                            "15",
                            "INV1",
                            "0012",
                            "A03-洗衣机",
                            "卡萨帝洗衣机C1",
                        ]),
                        row(&["1-04", "B1", "", "OX", "", "OTHER", "1", "1", "INV1", ""]),
                        row(&[
                            "2-006",
                            "B1",
                            "",
                            "O1",
                            "",
                            m,
                            "100",
                            "15",
                            "INV1",
                            "",
                            "A04-空调",
                            "格兰仕空调",
                        ]),
                    ],
                ),
                ("汇总", &["核销商编"], vec![row(&[m])]),
                (
                    "第七批",
                    &NEW_APPLIANCE,
                    vec![
                        row(&[
                            "7-01",
                            "B7",
                            "7.19-8.4",
                            "2026-01-01 00:00:00",
                            "2026-02-02 08:00:00",
                            "O2",
                            m,
                            "200",
                            "-30",
                            "199",
                            "INV2",
                            "已有买方",
                        ]),
                        row(&["", "B7", "", "", "", "O3", m, "1", "1", "", "MISS", ""]),
                        row(&["7-01", "B7", "", "", "", "O4", m, "1", "1", "", "", ""]),
                    ],
                ),
            ],
        );
        write_invoices(
            &dir,
            "发票_20260101.xlsx",
            &[("INV1", "旧文件买方", "1", ""), ("INV2", "", "", "已红冲")],
        );
        write_invoices(
            &dir,
            "发票_20260201.xlsx",
            &[
                ("INV1", "买方一", "100.5", "已红冲"),
                ("INV2", "买方二", "300", "开票完成"),
                ("", "空号", "9", "已红冲"),
            ],
        );
        let output = dir.join("家电台账.xlsx");
        generate_appliance(&dir, &output).unwrap();

        let (name, rows) = read_output(&output);
        assert_eq!(name, "家电台账");
        assert_eq!(text(&rows[0][0]), APPLIANCE.title);
        assert_eq!(
            rows[1].iter().map(text).collect::<Vec<_>>(),
            APPLIANCE.fields
        );
        let col = |field| position(&APPLIANCE, field);
        let data = &rows[2..];
        // 源顺序、重复记录及负金额保留；其他商户及汇总表排除。
        assert_eq!(
            data.iter()
                .map(|r| text(&r[col("商户订单号")]))
                .collect::<Vec<_>>(),
            ["O1", "O1", "O2", "O3", "O4"]
        );
        assert_eq!(
            data.iter()
                .map(|r| text(&r[col("序号")]))
                .collect::<Vec<_>>(),
            // 源序号原样保留：非连续、前导零、重复；空单元格留空。
            ["1-03", "2-006", "7-01", "", "7-01"]
        );
        assert_eq!(data[2][col("补贴金额")], Data::Float(-30.0));
        assert_eq!(data[0][col("销售金额")], Data::Float(100.0));
        assert_eq!(text(&data[0][col("销售企业名称")]), "企业甲");
        assert_eq!(text(&data[0][col("核销商编")]), m);
        assert_eq!(text(&data[0][col("交易参考号")]), "0012");
        // 品牌品类由商品名称与编码品类共同提取；无唯一品牌只填品类；无编码品类留空。
        assert_eq!(text(&data[0][col("品牌品类")]), "海尔洗衣机");
        assert_eq!(text(&data[1][col("品牌品类")]), "空调");
        assert_eq!(data[2][col("品牌品类")], Data::Empty);
        // 旧表交易时间映射为交易完成时间；新表优先同名字段；创建时间缺失或非日期文本原样。
        assert!(matches!(data[0][col("交易完成时间")], Data::DateTime(_)));
        assert_eq!(
            data[0][col("交易完成时间")]
                .as_datetime()
                .unwrap()
                .to_string(),
            "2026-01-01 10:00:00"
        );
        assert_eq!(
            data[2][col("交易完成时间")]
                .as_datetime()
                .unwrap()
                .to_string(),
            "2026-02-02 08:00:00"
        );
        assert_eq!(data[0][col("创建时间")], Data::Empty);
        assert_eq!(text(&data[2][col("创建时间")]), "7.19-8.4");
        // 发票按最新文件逐列补空，已有值不覆盖，未匹配及空号码留空。
        assert_eq!(data[0][col("发票金额")], Data::Float(100.5));
        assert_eq!(text(&data[0][col("发票头/购买方名称")]), "买方一");
        assert_eq!(data[2][col("发票金额")], Data::Float(199.0));
        assert_eq!(text(&data[2][col("发票头/购买方名称")]), "已有买方");
        for unmatched in &data[3..] {
            assert_eq!(unmatched[col("发票金额")], Data::Empty);
            assert_eq!(unmatched[col("发票头/购买方名称")], Data::Empty);
        }
        // 仅最新发票文件中开票状态为已红冲的号码填写发票状态。
        assert_eq!(
            data.iter()
                .map(|r| text(&r[col("发票状态")]))
                .collect::<Vec<_>>(),
            ["已红冲", "已红冲", "", "", ""]
        );

        write_invoices(
            &dir,
            "发票_20260301.xlsx",
            &[("INV1", "甲", "1", ""), ("INV1", "乙", "2", "")],
        );
        let error = generate_appliance(&dir, &output).unwrap_err();
        assert!(error.contains("重复") && error.contains("INV1"), "{error}");
        std::fs::remove_file(dir.join("发票_20260301.xlsx")).unwrap();
        std::fs::remove_file(dir.join("发票_20260201.xlsx")).unwrap();
        std::fs::remove_file(dir.join("发票_20260101.xlsx")).unwrap();
        assert!(
            generate_appliance(&dir, &output)
                .unwrap_err()
                .contains("发票_yyyymmdd")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn digital_ledger_leaves_missing_transition_columns_blank_and_numbers_across_sheets() {
        let dir = unique_temp_path("ledger-digital");
        std::fs::create_dir_all(&dir).unwrap();
        let m = MERCHANT_DIGITAL;
        let old_header = [
            "拨付批次",
            "交易时间",
            "参考号",
            "商户订单号",
            "销方名称",
            "经销商编号",
            "应收销售金额",
            "补贴销售金额",
            "品类",
            "商品明细",
            "券号_旧",
            "发票号码",
        ];
        let new_header = [
            "序号",
            "拨付批次",
            "提交时间",
            "商户订单号",
            "商户编号",
            "核销日期",
            "销售金额",
            "补贴金额",
            "补贴比例/补贴限额",
            "券号",
            "类别",
            "ID",
        ];
        let old_rows = (1..=98)
            .map(|n| {
                row(&[
                    "旧",
                    "2026-01-01 10:00:00",
                    "R",
                    &format!("O{n}"),
                    "企业",
                    m,
                    "10",
                    "1",
                    "手机",
                    "商品",
                    "X",
                    "INV",
                ])
            })
            .collect();
        let new_rows = vec![
            row(&[
                "9-01",
                "新",
                "2026-08-01 14:00:00",
                "N1",
                m,
                "2026-07-02",
                "100",
                "15",
                "0.15",
                "Q1",
                "平板",
                "ID1",
            ]),
            row(&[
                "9-03", "新", "", "N2", m, "", "100", "500", "500元", "Q2", "", "",
            ]),
            row(&["", "新", "", "N3", m, "", "", "", "", "", "", ""]),
        ];
        write_book(
            &dir.join("2026年数码补贴明细已经回款下发.xlsx"),
            0,
            &[
                ("过渡期", &old_header, old_rows),
                ("第九批", &new_header, new_rows),
            ],
        );
        write_invoices(
            &dir,
            "发票_20260201.xlsx",
            &[("INV", "购方甲", "", "已红冲")],
        );
        let output = dir.join("数码台账.xlsx");
        generate_digital(&dir, &output).unwrap();

        let (name, rows) = read_output(&output);
        assert_eq!(name, "数码台账");
        assert_eq!(text(&rows[0][0]), DIGITAL.title);
        assert_eq!(rows[1].iter().map(text).collect::<Vec<_>>(), DIGITAL.fields);
        let col = |field| position(&DIGITAL, field);
        let data = &rows[2..];
        assert_eq!(data.len(), 101);
        // 源序号随记录读取：旧表缺列留空，新表原样保留，空单元格留空，不补号。
        assert_eq!(
            data[97..]
                .iter()
                .map(|r| text(&r[col("序号")]))
                .collect::<Vec<_>>(),
            ["", "9-01", "9-03", ""]
        );
        let missing = [
            "序号",
            "提交时间",
            "门店名称/县域编码",
            "核销日期",
            "补贴比例/补贴限额",
            "原始SN码",
            "券号/数据唯一标识",
            "物流单号",
            "ID",
            "客户购买品类",
        ];
        for field in missing {
            assert_eq!(data[0][col(field)], Data::Empty, "{field}");
        }
        assert_eq!(text(&data[0][col("销售企业名称")]), "企业");
        assert_eq!(text(&data[0][col("商户编号")]), m);
        assert_eq!(text(&data[0][col("类别")]), "手机");
        assert_eq!(text(&data[0][col("商品名称")]), "商品");
        assert_eq!(text(&data[0][col("参考号")]), "R");
        assert_eq!(data[0][col("补贴金额")], Data::Float(1.0));
        let first_new = &data[98];
        assert!(matches!(first_new[col("提交时间")], Data::DateTime(_)));
        assert_eq!(
            first_new[col("核销日期")].as_date().unwrap().to_string(),
            "2026-07-02"
        );
        assert_eq!(first_new[col("补贴比例/补贴限额")], Data::Float(0.15));
        assert_eq!(text(&first_new[col("券号/数据唯一标识")]), "Q1");
        assert_eq!(text(&data[99][col("补贴比例/补贴限额")]), "500元");
        assert_eq!(data[100][col("销售金额")], Data::Empty);
        // 按发票号码关联开票状态；不补充发票金额等其他字段。
        assert_eq!(text(&data[0][col("发票状态")]), "已红冲");
        assert_eq!(data[98][col("发票状态")], Data::Empty);
        // 购买方名称只来自发票：按发票号码匹配，空号码留空。
        assert_eq!(text(&data[0][col("发票头/购买方名称")]), "购方甲");
        assert_eq!(data[98][col("发票头/购买方名称")], Data::Empty);
        std::fs::remove_file(dir.join("发票_20260201.xlsx")).unwrap();
        assert!(
            generate_digital(&dir, &output)
                .unwrap_err()
                .contains("发票_yyyymmdd")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ledger_groups_by_first_seen_batch_and_keeps_serials_when_batches_append() {
        let dir = unique_temp_path("ledger-batches");
        std::fs::create_dir_all(&dir).unwrap();
        let m = MERCHANT_DIGITAL;
        let header = ["拨付批次", "商户编号", "商户订单号", "序号"];
        // 同一批次分散在两张表；序号重复（O3、O6 均为 0-01）。
        let first = vec![
            row(&["B2", m, "O1", "2-01"]),
            row(&[" B1 ", m, "O2", "1-01"]),
            row(&["", m, "O3", "0-01"]),
            row(&["B2", m, "O4", "2-02"]),
        ];
        let mut second = vec![row(&["B1", m, "O5", "1-02"]), row(&["", m, "O6", "0-01"])];
        second.extend((7..=102).map(|n| row(&["B2", m, &format!("O{n}"), &format!("2-{n:03}")])));
        let source = dir.join("2026年数码补贴明细已经回款下发.xlsx");
        write_book(
            &source,
            0,
            &[
                ("一", &header, first.clone()),
                ("二", &header, second.clone()),
            ],
        );
        write_invoices(&dir, "发票_20260201.xlsx", &[]);
        let output = dir.join("数码台账.xlsx");
        generate_digital(&dir, &output).unwrap();

        let col = |field| position(&DIGITAL, field);
        let before = read_output(&output).1[2..].to_vec();
        let orders = |rows: &[Vec<Data>]| {
            rows.iter()
                .map(|r| text(&r[col("商户订单号")]))
                .collect::<Vec<_>>()
        };
        let pairs = |rows: &[Vec<Data>]| {
            rows.iter()
                .map(|r| (text(&r[col("商户订单号")]), text(&r[col("序号")])))
                .collect::<Vec<_>>()
        };
        let source_pairs = |sheets: &[&Vec<Vec<String>>]| {
            sheets
                .iter()
                .flat_map(|rows| rows.iter().map(|r| (r[2].clone(), r[3].clone())))
                .collect::<HashMap<_, _>>()
        };
        // 按首次出现顺序 B2、B1、空批次分组，不按名称排序；批次内保持源表及源行顺序。
        let mut expected = vec!["O1".to_string(), "O4".to_string()];
        expected.extend((7..=102).map(|n| format!("O{n}")));
        expected.extend(["O2", "O5", "O3", "O6"].map(String::from));
        assert_eq!(orders(&before), expected);
        // 序号随所属记录移动，分组不串号、不重新编号。
        let expected_pairs = source_pairs(&[&first, &second]);
        for (order, serial) in pairs(&before) {
            assert_eq!(serial, expected_pairs[&order], "{order}");
        }
        assert_eq!(
            pairs(&before[..2]),
            [("O1".into(), "2-01".into()), ("O4".into(), "2-02".into())]
        );

        // 源数据末尾追加新批次：原记录顺序、序号、内容不变，新记录接在末尾。
        second.extend([
            row(&["B3", m, "O103", "3-01"]),
            row(&["B3", m, "O104", "3-01"]),
        ]);
        write_book(
            &source,
            0,
            &[("一", &header, first), ("二", &header, second)],
        );
        generate_digital(&dir, &output).unwrap();
        let after = read_output(&output).1[2..].to_vec();
        assert_eq!(after[..102], before[..]);
        assert_eq!(
            pairs(&after[102..]),
            [
                ("O103".into(), "3-01".into()),
                ("O104".into(), "3-01".into())
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 表头不在第1行时按回款清洗的同一规则定位，表头行本身不作为明细。
    #[test]
    fn ledger_reads_below_located_header_row() {
        let dir = unique_temp_path("ledger-header-row");
        std::fs::create_dir_all(&dir).unwrap();
        let m = MERCHANT_DIGITAL;
        write_book(
            &dir.join("2026年数码补贴明细已经回款下发.xlsx"),
            1,
            &[(
                "批次",
                &["序号", "拨付批次", "商户编号", "商户订单号"],
                vec![
                    row(&["9", "B1", m, "O1"]),
                    row(&["10", "B1", "OTHER", "O2"]),
                ],
            )],
        );
        write_invoices(&dir, "发票_20260201.xlsx", &[]);
        let output = dir.join("数码台账.xlsx");
        generate_digital(&dir, &output).unwrap();
        let rows = read_output(&output).1;
        assert_eq!(rows.len(), 3);
        let col = |field| position(&DIGITAL, field);
        assert_eq!(text(&rows[2][col("商户订单号")]), "O1");
        assert_eq!(text(&rows[2][col("序号")]), "9");

        // 数值型源序号按现有文本转换写为完整整数文本。
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        for (col, field) in ["序号", "拨付批次", "商户编号", "商户订单号"]
            .iter()
            .enumerate()
        {
            ws.write_string(0, col as u16, *field).unwrap();
        }
        ws.write_number(1, 0, 12.0).unwrap();
        for (col, value) in ["B1", m, "O1"].iter().enumerate() {
            ws.write_string(1, col as u16 + 1, *value).unwrap();
        }
        book.save(dir.join("2026年数码补贴明细已经回款下发.xlsx"))
            .unwrap();
        generate_digital(&dir, &output).unwrap();
        assert_eq!(text(&read_output(&output).1[2][col("序号")]), "12");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn brand_category_combines_name_and_code_category() {
        let cases = [
            // 品牌在名称首、尾；`*品类*`前缀；大小写不敏感。
            ("格力空调KFR-35GW", "A04-空调", "格力空调"),
            ("XQG100-606H海尔", "A03-洗衣机", "海尔洗衣机"),
            ("*空调*美的空调 KFR-35GW/N8WF1", "A04-空调", "美的空调"),
            ("leader洗衣机", "A03-洗衣机", "海尔洗衣机"),
            // 归并。
            ("卡萨帝洗衣机", "A03-洗衣机", "海尔洗衣机"),
            ("统帅冰箱", "A02-电冰箱", "海尔电冰箱"),
            ("A.O.史密斯热水器", "A06-热水器", "AO史密斯热水器"),
            ("Vidda电视", "A01-电视机", "海信电视机"),
            ("华凌KFR-35GW", "A04-空调", "美的空调"),
            ("东芝冰箱", "A02-电冰箱", "美的电冰箱"),
            ("小天鹅洗衣机", "A03-洗衣机", "美的洗衣机"),
            ("COLMOKFR-35GW", "A04-空调", "美的空调"),
            // 5 个完整型号；相似型号不推断。
            ("TNDD20-08AIDE", "A03-洗衣机", "美的洗衣机"),
            ("65A6Q", "A01-电视机", "海信电视机"),
            ("85A6Q", "A01-电视机", "海信电视机"),
            ("KFR-50GW/CX2S", "A04-空调", "美的空调"),
            ("VL5R-PRO", "A01-电视机", "海信电视机"),
            ("65A6Q-PRO", "A01-电视机", "电视机"),
            // 品牌出现在编码品类中。
            ("XQB100", "A03-小天鹅洗衣机", "美的洗衣机"),
            // 无唯一品牌只填品类；编码品类无`-`时取整格。
            ("小鸭洗衣机XQB120-TD15EVB", "A03-洗衣机", "小鸭洗衣机"),
            ("科龙空调KFR-72LW/LX1-X1A", "A04-空调", "科龙空调"),
            ("格兰仕洗衣机XQB120", "A03-洗衣机", "洗衣机"),
            ("海尔洗衣机美的", "A03-洗衣机", "洗衣机"),
            ("方太BCD-508W-W", "A02-电冰箱", "方太电冰箱"),
            ("海信电视", "电视机", "海信电视机"),
        ];
        for (name, category, expected) in cases {
            assert_eq!(
                brand_category(name, category).as_deref(),
                Some(expected),
                "{name} / {category}"
            );
        }
        assert_eq!(brand_category("海尔洗衣机", " "), None);
    }

    #[test]
    fn empty_selection_still_writes_title_and_header_and_missing_merchant_field_fails() {
        let dir = unique_temp_path("ledger-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("2026年数码补贴明细已经回款下发.xlsx");
        write_book(
            &source,
            0,
            &[("批次", &["商户编号"], vec![row(&["OTHER"])])],
        );
        write_invoices(&dir, "发票_20260201.xlsx", &[]);
        let output = dir.join("数码台账.xlsx");
        generate_digital(&dir, &output).unwrap();
        assert_eq!(read_output(&output).1.len(), 2);

        write_book(
            &source,
            0,
            &[("批次", &["核销商编"], vec![row(&[MERCHANT_DIGITAL])])],
        );
        let error = generate_digital(&dir, &output).unwrap_err();
        assert!(error.contains("缺少商户筛选字段"), "{error}");

        // 同一字段的两个同义表头同时出现：结构异常，不任取一列。
        write_book(
            &source,
            0,
            &[(
                "批次",
                &["拨付批次", "商户编号", "经销商编号"],
                vec![row(&["B1", MERCHANT_DIGITAL, MERCHANT_DIGITAL])],
            )],
        );
        let error = generate_digital(&dir, &output).unwrap_err();
        assert!(error.contains("多个匹配"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
