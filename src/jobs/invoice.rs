use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::io::paths::list_xlsx_files;
use crate::io::xlsx_reader::open_sheets;
use crate::model::{Column, ColumnType, Fill, ProcessError, Row, Table, Value};
use crate::utils::{dates, doc_no};

use super::{
    Category, Job, PipelineContext, cell_text, data_error, parse_datetime_field,
    pick_unique_latest, text_value,
};

pub(crate) const SOURCE_HEADERS: [&str; 30] = [
    "订单号",
    "创建时间",
    "开票时间",
    "开票类型",
    "发票种类",
    "发票性质",
    "发票代码",
    "发票号码",
    "数电发票号码",
    "购方名称",
    "购方税号",
    "购方手机号",
    "购方邮箱",
    "购方开户行及账号",
    "购方地址、电话",
    "主要商品名称",
    "合计含税金额",
    "税率",
    "合计不含税金额",
    "合计税额",
    "备注信息",
    "部门门店",
    "开票方式",
    "开票员",
    "收款人",
    "复核人",
    "PDF地址",
    "开票状态",
    "操作人",
    "打印状态",
];

const OUTPUT_HEADERS: [&str; 10] = [
    "开票时间",
    "开票类型",
    "数电发票号码",
    "购方名称",
    "主要商品名称",
    "备注信息",
    "开票状态",
    "匹配单据号",
    "大类",
    "品牌",
];

pub(crate) struct InvoiceRecord {
    issue_time: Value,
    invoice_type: String,
    pub invoice_no: String,
    buyer_name: String,
    product_name: String,
    finance_category: String,
    brand: String,
    remark: String,
    invoice_status: String,
    pub match_doc_no: Value,
}

/// 文件名须完整符合`发票_yyyymmdd.xlsx`；返回 8 位数字部分。
fn invoice_date_digits(name: &str) -> Option<&str> {
    let digits = name.strip_prefix("发票_")?.strip_suffix(".xlsx")?;
    (digits.len() == 8 && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

/// 在`输入目录`中选择文件名日期最新的一个发票工作簿；不合并较早日期的文件。
fn select_latest_file(input_dir: &Path) -> Result<PathBuf, ProcessError> {
    let mut dated: Vec<(PathBuf, NaiveDate)> = Vec::new();

    for path in list_xlsx_files(input_dir)? {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(digits) = invoice_date_digits(name) else {
            continue;
        };
        match dates::parse_yyyymmdd(digits) {
            Some(date) => dated.push((path, date)),
            None => {
                return Err(ProcessError::Structure {
                    file: name.to_string(),
                    sheet: String::new(),
                    detail: format!("文件名日期无效：{digits}"),
                });
            }
        }
    }

    if dated.is_empty() {
        return Err(ProcessError::NoInput {
            pattern: "发票_yyyymmdd.xlsx".to_string(),
        });
    }

    pick_unique_latest(dated, "最新日期无法唯一确定：多个文件对应同一最新日期")
}

/// 以最后一个半角星号为分隔符，仅保留星号后的商品名称；不含星号时保持不变。
fn clean_product_name(raw: &str) -> String {
    raw.rsplit_once('*')
        .map_or(raw, |(_, name)| name)
        .to_string()
}

/// Extract the brand and category from invoice names; keep the full cleaned name intact.
/// Unknown categories and names without a confirmed brand remain blank.
fn split_product_name(name: &str) -> (String, String) {
    let Some((brand, remainder)) = name.split_once('-') else {
        return (String::new(), brand_from_unseparated_name(name));
    };
    let (category_token, _) = remainder.split_once('-').unwrap_or((remainder, ""));
    let category = invoice_finance_category(category_token).unwrap_or_default();
    (category.to_string(), normalize_invoice_brand(brand))
}

fn normalize_invoice_brand(brand: &str) -> String {
    match brand.to_ascii_lowercase().as_str() {
        "vivo" => "vivo".to_string(),
        "iqoo" => "iqoo".to_string(),
        "huawei" => "华为".to_string(),
        "oppo" => "OPPO".to_string(),
        "美的（小电）" | "美的（微清）" | "colmo厨热jx水系统" => {
            "美的".to_string()
        }
        _ => super::coupons::normalize_brand(brand.to_string()),
    }
}

fn brand_from_unseparated_name(name: &str) -> String {
    match name {
        "VIXO手机" | "手机vivo" => return "vivo".to_string(),
        "IQ手机" | "iQ手机" | "IQ00国产手机" => return "iqoo".to_string(),
        "Reno手机" => return "OPPO".to_string(),
        _ => {}
    }
    let lower_name = name.to_ascii_lowercase();
    [
        "OPPO",
        "vivo",
        "iqoo",
        "HUAWEI",
        "华为",
        "荣耀",
        "一加",
        "小米",
        "红米",
        "苹果",
        "小天才",
        "学而思",
        "作业帮",
        "海尔",
        "卡萨帝",
        "美的",
        "美菱",
        "小天鹅",
        "创维",
        "TCL",
        "海信",
        "晶弘",
        "奥克斯",
        "格力",
        "西门子",
        "欧意",
        "沁园",
        "方太",
        "长城",
        "COLMO",
        "统帅",
        "步步高",
        "老板",
    ]
    .into_iter()
    .find(|brand| lower_name.starts_with(&brand.to_ascii_lowercase()))
    .map(normalize_invoice_brand)
    .unwrap_or_default()
}

fn invoice_finance_category(token: &str) -> Option<&'static str> {
    Some(match token {
        "国产手机" | "合资手机" | "学习机" | "电话手表" | "手表" => "数码",
        "空调"
        | "家用挂机直流无氟变频1.5P35"
        | "家用柜机直流无氟变频3P72"
        | "风管机"
        | "中央空调多联机"
        | "家用挂机直流无氟变频2P50"
        | "家用柜机直流无氟变频4P"
        | "家用多联机外机"
        | "家用挂机直流无氟变频1P26"
        | "家用挂机直流无氟变频3P"
        | "家用柜机直流无氟变频5P" => "空调",
        "洗衣机" | "洗鞋一体机" => "洗衣机",
        "干衣机" => "洗衣机",
        "冰箱" | "冷柜" | "冰吧" | "冰柜" => "冰箱",
        "电热水器"
        | "燃气热水器"
        | "欧式烟机"
        | "嵌入式灶"
        | "热水器其他"
        | "欧式烟机CXW"
        | "水系统"
        | "电器"
        | "进吸式烟机"
        | "洗碗机cw"
        | "洗碗机（厨卫）"
        | "岛式烟机"
        | "即热式热水器"
        | "CWS"
        | "嵌入式消毒柜"
        | "嵌入式蒸箱\\烤箱\\微波炉" => "厨卫",
        "彩电"
        | "国产普通LED75寸"
        | "国产普通LED85寸"
        | "国产普通"
        | "国产普通LED100寸"
        | "国产普通LED98寸"
        | "激光投影"
        | "彩电艺术显示"
        | "国产普通LED86寸"
        | "国产激光4K88寸"
        | "国产激光4K80寸"
        | "国产激光LED100寸"
        | "国产激光LED86寸"
        | "国产艺术电视LED75寸"
        | "壁纸电视"
        | "国产壁纸电视LED85寸"
        | "艺术显示彩电" => "彩电",
        "投影仪" => "彩电",
        "净水类"
        | "净水类（小电）"
        | "净水机"
        | "洁净类"
        | "其他生活小电"
        | "电解水机"
        | "锅具"
        | "炉具"
        | "加工机"
        | "智能锁" => "小电",
        "多联空调" | "多联内机" | "厨房空调器" | "厨房空调" | "多联机配件（控制器）" => {
            "空调"
        }
        "国产艺术电视LED85寸" => "彩电",
        _ => return None,
    })
}

fn is_abnormal(record: &InvoiceRecord) -> bool {
    record.invoice_type == "红票"
        || record.invoice_status == "已红冲"
        || record.invoice_status == "开票失败"
}

pub struct InvoiceJob;

impl Job for InvoiceJob {
    fn category(&self) -> Category {
        Category::Invoice
    }

    fn title(&self) -> &'static str {
        "发票明细"
    }

    fn output_stem(&self) -> &'static str {
        "发票明细"
    }

    fn run_in_context(
        &self,
        input_dir: &Path,
        ctx: &mut PipelineContext,
    ) -> Result<Table, ProcessError> {
        let records = load_records(input_dir)?;
        ctx.store_invoices(&records);
        Ok(classify_and_build_table(records))
    }
}

/// 读取当前最新发票工作簿的全部有效明细（含正常、重复与异常记录），供本任务
/// 及第 10 节`数电发票号码`匹配复用；不做分类、排序或填色。
pub(crate) fn load_records(input_dir: &Path) -> Result<Vec<InvoiceRecord>, ProcessError> {
    let path = select_latest_file(input_dir)?;
    let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

    let sheets = open_sheets(&path)?;
    if sheets.len() != 1 {
        return Err(ProcessError::Structure {
            file: file_name,
            sheet: String::new(),
            detail: format!("工作表数量异常：应为 1 个，实际为 {} 个", sheets.len()),
        });
    }
    let sheet = &sheets[0];
    let sheet_name = sheet.name().to_string();

    let header = sheet.row_texts(6);
    if header.iter().map(String::as_str).collect::<Vec<_>>() != SOURCE_HEADERS {
        return Err(ProcessError::Structure {
            file: file_name,
            sheet: sheet_name,
            detail: format!("第6行表头与规定的30个字段不一致：{header:?}"),
        });
    }

    let last_row = sheet.last_value_row().unwrap_or(0);

    let mut records = Vec::new();
    for row in 7..=last_row {
        let issue_time = parse_datetime_field(
            &sheet.cell(row, 3),
            "开票时间",
            &file_name,
            &sheet_name,
            row,
        )?;

        let text_at = |col: u32, field: &'static str| -> Result<String, ProcessError> {
            let cell = sheet.cell(row, col);
            cell_text(&cell).map_err(|detail| {
                data_error(
                    &file_name,
                    &sheet_name,
                    row,
                    field,
                    cell.to_string(),
                    detail,
                )
            })
        };

        let invoice_type = text_at(4, "开票类型")?;
        let invoice_no = text_at(9, "数电发票号码")?;
        let buyer_name = text_at(10, "购方名称")?;
        let raw_product_name = text_at(16, "主要商品名称")?;
        let product_name = clean_product_name(&raw_product_name);
        let (finance_category, brand) = split_product_name(&product_name);
        let remark = text_at(21, "备注信息")?;
        let invoice_status = text_at(28, "开票状态")?;

        records.push(InvoiceRecord {
            issue_time,
            invoice_type,
            invoice_no,
            buyer_name,
            product_name,
            finance_category,
            brand,
            match_doc_no: doc_no::MatchDocNo::from_remark(&remark)
                .map_or(Value::Empty, |m| Value::Text(m.into_string())),
            remark,
            invoice_status,
        });
    }

    Ok(records)
}

fn classify_and_build_table(records: Vec<InvoiceRecord>) -> Table {
    let mut abnormal = Vec::new();
    let mut candidates = Vec::new();
    for record in records {
        if is_abnormal(&record) {
            abnormal.push(record);
        } else {
            candidates.push(record);
        }
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for record in &candidates {
        if let Value::Text(no) = &record.match_doc_no {
            *counts.entry(no.clone()).or_insert(0) += 1;
        }
    }

    let mut normal = Vec::new();
    let mut duplicate = Vec::new();
    for record in candidates {
        let is_duplicate = matches!(&record.match_doc_no, Value::Text(no) if counts.get(no).copied().unwrap_or(0) > 1);
        if is_duplicate {
            duplicate.push(record);
        } else {
            normal.push(record);
        }
    }

    let rows = normal
        .into_iter()
        .map(|record| to_row(record, None))
        .chain(
            duplicate
                .into_iter()
                .map(|record| to_row(record, Some(Fill::Yellow))),
        )
        .chain(
            abnormal
                .into_iter()
                .map(|record| to_row(record, Some(Fill::Pink))),
        )
        .collect();

    Table {
        columns: output_columns(),
        rows,
    }
}

fn output_columns() -> Vec<Column> {
    const TYPES: [ColumnType; 10] = [
        ColumnType::DateTime,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
    ];
    OUTPUT_HEADERS
        .into_iter()
        .zip(TYPES)
        .map(|(name, ty)| Column { name, ty })
        .collect()
}

fn to_row(record: InvoiceRecord, fill: Option<Fill>) -> Row {
    let values = vec![
        record.issue_time,
        text_value(record.invoice_type),
        text_value(record.invoice_no),
        text_value(record.buyer_name),
        text_value(record.product_name),
        text_value(record.remark),
        text_value(record.invoice_status),
        record.match_doc_no,
        text_value(record.finance_category),
        text_value(record.brand),
    ];
    Row { values, fill }
}

#[cfg(test)]
mod tests {
    use rust_xlsxwriter::Workbook;

    use super::*;
    use crate::test_support::unique_temp_path;

    #[test]
    fn cleans_product_name_at_last_asterisk() {
        assert_eq!(
            clean_product_name("*家用清洁电器具*小天鹅-洗衣机-TG12TP3"),
            "小天鹅-洗衣机-TG12TP3"
        );
        assert_eq!(clean_product_name("无星号商品"), "无星号商品");
    }

    fn write_invoice_workbook(path: &Path, sheet_name: &str, rows: &[[&str; 30]]) {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.set_name(sheet_name).unwrap();
        for (col, header) in SOURCE_HEADERS.iter().enumerate() {
            sheet.write_string(5, col as u16, *header).unwrap();
        }
        for (index, row) in rows.iter().enumerate() {
            for (col, value) in row.iter().enumerate() {
                sheet
                    .write_string((6 + index) as u32, col as u16, *value)
                    .unwrap();
            }
        }
        workbook.save(path).unwrap();
    }

    fn sample_source_row<'a>(
        issue_time: &'a str,
        invoice_type: &'a str,
        invoice_status: &'a str,
        remark: &'a str,
        product_name: &'a str,
    ) -> [&'a str; 30] {
        [
            "ORDER001",
            "2026-09-14 09:00:00",
            issue_time,
            invoice_type,
            "数电发票",
            "正常发票",
            "",
            "",
            "24312000000000000001",
            "张三",
            "",
            "",
            "",
            "",
            "",
            product_name,
            "100.00",
            "13%",
            "88.50",
            "11.50",
            remark,
            "总店",
            "自动开票",
            "开票员甲",
            "收款人甲",
            "复核人甲",
            "",
            invoice_status,
            "操作人甲",
            "已打印",
        ]
    }

    #[test]
    fn alternate_document_labels_survive_invoice_cleaning() {
        let dir = unique_temp_path("invoice-alternate-labels");
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            (
                "购机日期：2026.2.27 单据号收款：ZHFH000125",
                "260227ZHFH000125",
            ),
            (
                "销售日期、2026-01-18单据号收款号、ZFP3000105",
                "260118ZFP3000105",
            ),
        ];
        for (remark, expected) in cases {
            write_invoice_workbook(
                &dir.join("发票_20260914.xlsx"),
                "发票_20260914",
                &[sample_source_row(
                    "2026-01-25 10:00:00",
                    "蓝票",
                    "开票完成",
                    remark,
                    "商品",
                )],
            );
            let table = InvoiceJob.run(&dir).unwrap();
            assert_eq!(
                table.rows[0].values[7],
                Value::Text(expected.into()),
                "{remark}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interleaved_padding_is_corrected_without_relaxing_invoice_guards() {
        let dir = unique_temp_path("invoice-interleaved-padding");
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            (
                "销售日期:2026-01-03 单据号:收款ZHA40000013",
                Some("260103ZHA4000013"),
            ),
            (
                "销售日期:2026-05-31 单据号:收款ZH700000103",
                Some("260531ZH70000103"),
            ),
            (
                "销售日期:2026-05-31 单据号:收款ZH700000102",
                Some("260531ZH70000102"),
            ),
            (
                "销售日期:2026-05-31 单据号:收款ZH700000101",
                Some("260531ZH70000101"),
            ),
            ("销售日期:2026-05-31 单据号:收款ZH712345678", None),
            ("销售日期:2026-05-31 单据号:收款ZH7000023", None),
            ("单据号收款：ZH700000103", None),
            (
                "销售日期:2026-05-31 单据号收款：ZH700000103 单据号:ZH700000102",
                None,
            ),
        ];
        for (remark, expected) in cases {
            write_invoice_workbook(
                &dir.join("发票_20260914.xlsx"),
                "发票_20260914",
                &[sample_source_row(
                    "2026-01-25 10:00:00",
                    "蓝票",
                    "开票完成",
                    remark,
                    "商品",
                )],
            );
            let table = InvoiceJob.run(&dir).unwrap();
            assert_eq!(
                table.rows[0].values[7],
                expected.map_or(Value::Empty, |value| Value::Text(value.into())),
                "{remark}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn interleaved_document_number_survives_invoice_cleaning() {
        let dir = unique_temp_path("invoice-interleaved-document");
        std::fs::create_dir_all(&dir).unwrap();
        write_invoice_workbook(
            &dir.join("发票_20260914.xlsx"),
            "发票_20260914",
            &[sample_source_row(
                "2026-01-25 10:00:00",
                "蓝票",
                "开票完成",
                "销售日期:2026-01-25 单据号:收款ZG2J000424",
                "VIVO-国产手机",
            )],
        );
        let table = InvoiceJob.run(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(
            table.rows[0].values[7],
            Value::Text("260125ZG2J000424".into())
        );
    }

    #[test]
    fn interleaved_documents_keep_date_length_and_uniqueness_rules() {
        let dir = unique_temp_path("invoice-interleaved-guards");
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            (
                "购机日期:2026-01-25 单据收款号:ZG2J000424",
                "valid",
                Some("260125ZG2J000424"),
            ),
            (
                "销售日期:2026-01-25 单据号:收款ZG2J000424 单据号:收款ZG2J000425",
                "ambiguous-document",
                None,
            ),
            (
                "销售日期:2026-01-25 购机日期:2026-01-26 单据号:收款ZG2J000424",
                "ambiguous-date",
                None,
            ),
            ("单据号:收款ZG2J000424", "missing-date", None),
            (
                "销售日期:2026-01-25 单据号:收款ZG2J00042",
                "short-document",
                None,
            ),
            (
                "销售日期:2026-01-25 单据号:收款ZG2J000424AB",
                "long-document",
                None,
            ),
            ("销售日期:2026-01-25 单据号:ABCDEFGHIJ", "no-digits", None),
            (
                "销售日期:2026-01-25 单据号:收款ZG2J000424 单据号:收款ZG2J000424",
                "same-document",
                Some("260125ZG2J000424"),
            ),
        ];
        let source: Vec<_> = cases
            .iter()
            .map(|(remark, name, _)| {
                sample_source_row("2026-01-25 10:00:00", "蓝票", "开票完成", remark, name)
            })
            .collect();
        write_invoice_workbook(&dir.join("发票_20260914.xlsx"), "发票_20260914", &source);
        let table = InvoiceJob.run(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(table.rows.len(), cases.len());
        for (_, name, expected) in cases {
            let row = table
                .rows
                .iter()
                .find(|row| row.values[4] == Value::Text(name.into()))
                .unwrap();
            assert_eq!(
                row.values[7],
                expected.map_or(Value::Empty, |value| Value::Text(value.into())),
                "{name}"
            );
        }
    }

    #[test]
    fn blank_issue_time_stays_empty_instead_of_erroring() {
        let dir = unique_temp_path("invoice-blank-issue-time");
        std::fs::create_dir_all(&dir).unwrap();
        write_invoice_workbook(
            &dir.join("发票_20260914.xlsx"),
            "发票_20260914",
            &[sample_source_row("", "蓝票", "开票完成", "", "商品甲")],
        );

        let table = InvoiceJob.run(&dir).unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].values[0], Value::Empty);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn product_fields_survive_cleaning_without_changing_full_names() {
        let dir = unique_temp_path("invoice-product-fields");
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            ("OPPO手机", "", "OPPO"),
            ("vivo手机", "", "vivo"),
            ("卡萨帝冰箱", "", "海尔"),
            ("Reno手机", "", "OPPO"),
            ("VIXO手机", "", "vivo"),
            ("IQ手机", "", "iqoo"),
            ("iQ手机", "", "iqoo"),
            ("IQ00国产手机", "", "iqoo"),
            ("手机", "", ""),
            ("平板", "", ""),
            ("手表", "", ""),
            ("家用平板电脑", "", ""),
            ("空调", "", ""),
            ("烟机灶具套餐", "", ""),
            ("美的（微清）-炉具-微波炉C237", "小电", "美的"),
            ("COLMO厨热JX水系统-CWS-F08", "厨卫", "美的"),
            ("小天鹅-干衣机-TH12B5", "洗衣机", "美的"),
            (
                "美的厨热JX-嵌入式蒸箱\\烤箱\\微波炉-BG50T5W",
                "厨卫",
                "美的",
            ),
            ("海尔-未知类别-型号-A", "", "海尔"),
        ];
        let raw_names: Vec<_> = cases
            .iter()
            .map(|(name, _, _)| format!("*商品*{name}"))
            .collect();
        let rows: Vec<_> = raw_names
            .iter()
            .map(|name| sample_source_row("2026-09-14 10:00:00", "蓝票", "开票完成", "", name))
            .collect();
        write_invoice_workbook(&dir.join("发票_20260914.xlsx"), "发票_20260914", &rows);
        let table = InvoiceJob.run(&dir).unwrap();
        assert_eq!(table.columns.len(), 10);
        assert_eq!(table.rows.len(), cases.len());
        for (row, (name, category, brand)) in table.rows.iter().zip(cases) {
            assert_eq!(row.values[4], Value::Text(name.into()));
            assert_eq!(row.values[8], text_value(category.into()), "{name}");
            assert_eq!(row.values[9], text_value(brand.into()), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn selects_latest_file_validates_and_classifies_records() {
        let dir = unique_temp_path("invoice-happy-path");
        std::fs::create_dir_all(&dir).unwrap();

        // 较早日期的文件必须被忽略。
        write_invoice_workbook(
            &dir.join("发票_20260913.xlsx"),
            "发票_20260913",
            &[sample_source_row(
                "2026-09-13 08:00:00",
                "蓝票",
                "开票完成",
                "销售日期:2026-09-13 单据号:收款OLD0000001",
                "*旧数据*不应出现",
            )],
        );

        write_invoice_workbook(
            &dir.join("发票_20260914.xlsx"),
            "发票_20260914",
            &[
                sample_source_row(
                    "2026-09-14 10:00:00",
                    "蓝票",
                    "开票完成",
                    "销售日期:2026-07-28 单据号:收款ZEXQ000062",
                    "*家用清洁电器具*小天鹅-洗衣机-TG12TP3",
                ),
                sample_source_row(
                    "2026-09-14 11:00:00",
                    "蓝票",
                    "开票完成",
                    "销售日期:2026-07-14 单据号:收款ZEXQ000099",
                    "重复单据商品甲",
                ),
                sample_source_row(
                    "2026-09-14 11:30:00",
                    "蓝票",
                    "开票完成",
                    "销售日期:2026-07-14 单据号:收款ZEXQ000099",
                    "重复单据商品乙",
                ),
                sample_source_row(
                    "2026-09-14 12:00:00",
                    "红票",
                    "开票完成",
                    "红冲",
                    "异常商品",
                ),
            ],
        );

        let job = InvoiceJob;
        let table = job.run(&dir).unwrap();

        assert_eq!(table.columns.len(), 10);
        assert_eq!(table.rows.len(), 4);

        // 正常记录：星号清洗后的商品名称，唯一匹配单据号，且不含旧文件数据。
        assert_eq!(
            table.rows[0].values[4],
            Value::Text("小天鹅-洗衣机-TG12TP3".to_string())
        );
        assert_eq!(table.rows[0].values[8], Value::Text("洗衣机".into()));
        assert_eq!(table.rows[0].values[9], Value::Text("美的".into()));
        assert_eq!(table.rows[0].fill, None);
        assert_eq!(
            table.rows[0].values[7],
            Value::Text("260728ZEXQ000062".to_string())
        );

        // 重复记录：匹配单据号相同的两条记录均归入重复组、黄色填充，位于正常记录之后。
        assert_eq!(table.rows[1].fill, Some(Fill::Yellow));
        assert_eq!(table.rows[2].fill, Some(Fill::Yellow));
        assert_eq!(
            table.rows[1].values[7],
            Value::Text("260714ZEXQ000099".to_string())
        );
        assert_eq!(
            table.rows[2].values[7],
            Value::Text("260714ZEXQ000099".to_string())
        );

        // 异常记录：红票，粉色填充，位于最底部。
        assert_eq!(table.rows[3].fill, Some(Fill::Pink));
        assert_eq!(table.rows[3].values[1], Value::Text("红票".to_string()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_header_mismatch() {
        let dir = unique_temp_path("invoice-bad-header");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("发票_20260914.xlsx");

        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        sheet.set_name("发票_20260914").unwrap();
        sheet.write_string(5, 0, "错误表头").unwrap();
        workbook.save(&path).unwrap();

        let error = InvoiceJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::Structure { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_invalid_filename_date() {
        let dir = unique_temp_path("invoice-bad-filename-date");
        std::fs::create_dir_all(&dir).unwrap();
        write_invoice_workbook(
            &dir.join("发票_20261399.xlsx"),
            "发票_20261399",
            &[sample_source_row(
                "2026-09-14 10:00:00",
                "蓝票",
                "开票完成",
                "",
                "",
            )],
        );

        let error = InvoiceJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::Structure { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_no_input_when_nothing_matches() {
        let dir = unique_temp_path("invoice-no-input");
        std::fs::create_dir_all(&dir).unwrap();

        let error = InvoiceJob.run(&dir).unwrap_err();
        assert!(matches!(error, ProcessError::NoInput { .. }));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
