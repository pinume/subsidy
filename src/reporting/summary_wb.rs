use calamine::Data;
use rust_decimal::Decimal;
use rust_xlsxwriter::{
    Color, ConditionalFormatFormula, Format, Workbook, Worksheet, column_number_to_name,
};
use std::path::Path;

use super::excel::{ReportError, ReportSheetBuilder};
use super::reader::{cell_to_string, get_colored_row_indices, get_colored_row_indices_multi};
use super::styles::StylePool;

use super::data::{CommonInputs, ReportColumn, SheetData};
#[cfg(test)]
use super::reader::read_sheet_rows;
use super::summary_data::{
    AnomalyColors, AnomalyDatasets, CategoryBrandSummary, FailedUploadRecord,
    InvoiceAnomalySection, MetricRow, RefundAnomalySection, SUMMARY_STATUSES, SummaryMetrics,
    build_invoice_name_index,
};
#[cfg(test)]
use super::unique_index::UniqueIndex;

/// Generate Workbook 1: 国补上传情况汇总.xlsx (5 Sheets)
pub fn generate_summary_workbook(input_dir: &Path, output_path: &Path) -> Result<(), String> {
    let inputs = CommonInputs::load(input_dir)?;
    generate_with_inputs(input_dir, output_path, &inputs).map(|_| ())
}

pub(crate) fn generate_with_inputs(
    input_dir: &Path,
    output_path: &Path,
    inputs: &CommonInputs,
) -> Result<Vec<String>, String> {
    let mut workbook = Workbook::new();
    let styles = StylePool::default();

    let sales_rows = &inputs.sales;
    let app_upload_rows = &inputs.app_upload;
    let dig_upload_rows = &inputs.dig_upload;
    let invoice_rows = &inputs.invoices;
    let app_refund_rows = &inputs.app_refund;
    let dig_refund_rows = &inputs.dig_refund;
    let app_refund_path = input_dir.join("回款明细家电电脑.xlsx");
    let dig_refund_path = input_dir.join("回款明细数码.xlsx");
    let app_pink_rows = get_colored_row_indices(&app_refund_path, "FFFFC7CE")?;
    let dig_pink_rows = get_colored_row_indices(&dig_refund_path, "FFFFC7CE")?;
    let invoice_path = input_dir.join("发票明细.xlsx");
    let mut inv_colors = get_colored_row_indices_multi(&invoice_path, &["FFFFEB9C", "FFFFC7CE"])?;
    let inv_yellow_rows = inv_colors.remove("FFFFEB9C").unwrap_or_default();
    let inv_pink_rows = inv_colors.remove("FFFFC7CE").unwrap_or_default();

    let app_store_code = &inputs.app_store_code;
    let dig_store_code = &inputs.dig_store_code;

    // Precalculate shared metrics and lookup structures once
    let metrics = SummaryMetrics::calculate(sales_rows, app_upload_rows, dig_upload_rows)?;
    let category_brands = CategoryBrandSummary::from_sales(sales_rows)?;
    let invoice_name_index = build_invoice_name_index(invoice_rows)?;

    let colors = AnomalyColors {
        app_pink: app_pink_rows,
        dig_pink: dig_pink_rows,
        invoice_yellow: inv_yellow_rows,
        invoice_pink: inv_pink_rows,
    };
    let datasets = AnomalyDatasets::extract(
        [
            app_upload_rows,
            dig_upload_rows,
            invoice_rows,
            app_refund_rows,
            dig_refund_rows,
        ],
        &colors,
        [app_store_code, dig_store_code],
        &invoice_name_index,
    )?;

    // Build Sheet 1: 汇总
    build_summary_sheet(&mut workbook, &styles, &metrics)?;

    // Build Sheet 2: 品类品牌汇总
    build_category_brand_sheet(&mut workbook, &styles, &category_brands)?;

    // Build Sheet 3: 审核失败明细
    build_failed_records_sheet(
        &mut workbook,
        &styles,
        app_upload_rows,
        dig_upload_rows,
        &datasets.app_failed,
        &datasets.dig_failed,
    )?;

    // Build Sheet 4: 异常回款明细
    build_refund_anomaly_sheet(
        &mut workbook,
        &styles,
        app_refund_rows,
        dig_refund_rows,
        &datasets.app_refund,
        &datasets.dig_refund,
    )?;

    // Build Sheet 5: 异常发票明细
    build_invoice_anomaly_sheet(&mut workbook, &styles, invoice_rows, &datasets.invoices)?;

    workbook
        .save(output_path)
        .map_err(|e| format!("保存工作簿一失败: {}", e))?;

    Ok([
        metrics.duplicate_warning(),
        invoice_name_index.warning("发票商品名称索引"),
    ]
    .into_iter()
    .flatten()
    .collect())
}

/// 两层表头：首列纵向合并，其后每个分组横向合并并在第二行写入子列名。
fn write_grouped_header(
    sheet: &mut ReportSheetBuilder<'_>,
    leading: &[&str],
    groups: &[&str],
    sub: [&str; 2],
) -> Result<(), ReportError> {
    let (top, s) = (sheet.cur_row, sheet.styles);
    sheet.ws.set_row_height(top, 30.0)?;
    sheet.ws.set_row_height(top + 1, 30.0)?;
    for (col, label) in leading.iter().enumerate() {
        let col = col as u16;
        sheet
            .ws
            .merge_range(top, col, top + 1, col, label, &s.col_header_center)?;
    }
    for (index, group) in groups.iter().enumerate() {
        let col = (leading.len() + index * 2) as u16;
        sheet
            .ws
            .merge_range(top, col, top, col + 1, group, &s.col_header_center)?;
        sheet
            .ws
            .write_string_with_format(top + 1, col, sub[0], &s.col_header_center)?;
        sheet
            .ws
            .write_string_with_format(top + 1, col + 1, sub[1], &s.col_header_center)?;
    }
    sheet.cur_row += 2;
    Ok(())
}

fn write_tip_row(sheet: &mut ReportSheetBuilder<'_>) -> Result<(), ReportError> {
    let row = sheet.cur_row;
    sheet.ws.set_row_height(row, 22.0)?;
    sheet
        .ws
        .merge_range(row, 0, row, 6, "未回款具体情况", &sheet.styles.tip_row)?;
    sheet.cur_row += 1;
    Ok(())
}

fn build_summary_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    m: &SummaryMetrics,
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "汇总", s)?;
    sheet
        .set_column_widths(&[38.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0])?
        .write_title_block(
            6,
            "国补上传情况汇总",
            "数据截止口径：按所提供源文件全量数据统计；金额单位：万元",
        )?
        .write_section_header(6, "一、金额与数量汇总")?;
    let groups = ["家电电脑", "数码", "合计"];
    write_grouped_header(&mut sheet, &["项目"], &groups, ["金额（万元）", "笔数"])?;

    // Convert Yuan to Wan Yuan
    let to_wan = |d: Decimal| d / Decimal::from(10000);

    let t1_data: [(&str, &MetricRow); 7] = [
        ("国补发生额", &m.occur),
        ("国补已回款额", &m.paid),
        ("未回款", &m.unpaid),
        ("审核通过未回款", &m.pass),
        ("待审核", &m.wait),
        ("审核失败", &m.fail),
        ("未上传", &m.unup),
    ];
    for (i, (name, row)) in t1_data.iter().enumerate() {
        if i == 3 {
            write_tip_row(&mut sheet)?;
        }
        let (r, ws) = (sheet.cur_row, &mut *sheet.ws);
        ws.set_row_height(r, 22.0)?;
        ws.write_string_with_format(r, 0, *name, &s.text_left)?;
        ws.write_with_format(r, 1, to_wan(row.app_amt), &s.money)?;
        ws.write_number_with_format(r, 2, row.app_cnt as f64, &s.int_count)?;
        ws.write_with_format(r, 3, to_wan(row.dig_amt), &s.money)?;
        ws.write_number_with_format(r, 4, row.dig_cnt as f64, &s.int_count)?;
        ws.write_with_format(r, 5, to_wan(row.tot_amt()), &s.money)?;
        ws.write_number_with_format(r, 6, row.tot_cnt() as f64, &s.int_count)?;
        sheet.cur_row += 1;
    }

    sheet
        .write_blank_row(8.0)?
        .write_section_header(6, "二、结构与比率分析")?;
    write_grouped_header(
        &mut sheet,
        &["指标"],
        &groups,
        ["金额（万元）", "占比/比率"],
    )?;

    let names = [
        "国补发生额及占比",
        "国补回款额及回款率",
        "回款+审核通过未回款及综合回款率",
        "未回款额",
        "审核通过未回款",
        "待审核",
        "审核失败",
        "未上传",
    ];
    for (i, (name, (a_amt, a_pct, d_amt, d_pct, t_amt, t_pct))) in
        names.iter().zip(m.amount_ratios()).enumerate()
    {
        if i == 4 {
            write_tip_row(&mut sheet)?;
        }
        let (r, ws) = (sheet.cur_row, &mut *sheet.ws);
        ws.set_row_height(r, 24.0)?;
        ws.write_string_with_format(r, 0, *name, &s.text_left)?;
        ws.write_with_format(r, 1, to_wan(a_amt), &s.money)?;
        ws.write_number_with_format(r, 2, a_pct, &s.percent)?;
        ws.write_with_format(r, 3, to_wan(d_amt), &s.money)?;
        ws.write_number_with_format(r, 4, d_pct, &s.percent)?;
        ws.write_with_format(r, 5, to_wan(t_amt), &s.money)?;
        ws.write_number_with_format(r, 6, t_pct, &s.percent)?;
        sheet.cur_row += 1;
    }

    sheet
        .write_blank_row(8.0)?
        .write_section_header(6, "口径说明")?;

    let notes = [
        "1. 国补发生额及发生数量取自《销售用券情况统计.xlsx》；数量按“数量”字段净额统计，包含退货负数冲减。",
        "2. “财务大类=数码”计入数码，其余财务大类计入家电电脑。",
        "3. 已回款、审核通过未回款、待审核、审核失败取自两份“已上传”明细；金额汇总“补贴金额”，数量按明细记录数（同一实时清分UUID的重复记录只计更新时间最新的状态）；审核终止不计入上述金额、笔数及失败明细。",
        "4. 未回款=国补发生额-国补回款额；未上传=未回款-审核通过未回款-待审核-审核失败。",
        "5. 回款率=国补回款额/国补发生额；综合回款率=(国补回款额+审核通过未回款额)/国补发生额。",
    ];
    for note in notes {
        let r = sheet.cur_row;
        sheet.ws.set_row_height(r, 28.0)?;
        sheet.ws.merge_range(r, 0, r, 6, note, &s.text_left)?;
        sheet.cur_row += 1;
    }

    Ok(())
}

fn build_category_brand_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    summary: &CategoryBrandSummary,
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "品类品牌汇总", s)?;
    let mut widths = vec![16.0, 20.0];
    widths.extend([16.0; 10]);
    sheet.set_column_widths(&widths)?.write_title_block(
        11,
        "品类品牌汇总",
        "数据源：销售用券情况统计.xlsx；已排除退货-原单、退货-退单；金额单位：元",
    )?;
    write_grouped_header(
        &mut sheet,
        &["财务大类", "品牌"],
        &SUMMARY_STATUSES,
        ["补贴金额（元）", "数量"],
    )?;

    let ws = &mut *sheet.ws;
    let mut row_idx = sheet.cur_row;
    for group in summary.groups() {
        let cat = &group.category;
        let start_row = row_idx;
        for brand in &group.brands {
            ws.set_row_height(row_idx, 22.0)?;
            ws.write_string_with_format(row_idx, 0, cat, &s.text_center)?;
            ws.write_string_with_format(row_idx, 1, brand.brand.as_str(), &s.text_left)?;

            for (s_idx, &(amt, cnt)) in brand.totals.iter().enumerate() {
                let col_amt = 2 + (s_idx as u16) * 2;
                let col_cnt = col_amt + 1;

                if amt.is_zero() && cnt == 0 {
                    ws.write_string_with_format(row_idx, col_amt, "-", &s.text_right)?;
                    ws.write_string_with_format(row_idx, col_cnt, "-", &s.text_right)?;
                } else {
                    ws.write_with_format(row_idx, col_amt, amt, &s.money)?;
                    ws.write_number_with_format(row_idx, col_cnt, cnt as f64, &s.int_count)?;
                }
            }
            row_idx += 1;
        }
        let end_row = row_idx - 1;
        if end_row > start_row {
            ws.merge_range(start_row, 0, end_row, 0, cat, &s.text_center)?;
        }
    }

    Ok(())
}

fn build_failed_records_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    app_up: &SheetData,
    dig_up: &SheetData,
    app_failed: &[FailedUploadRecord<'_>],
    dig_failed: &[FailedUploadRecord<'_>],
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "审核失败明细", s)?;

    // Column widths: A=14, B=20, C=16, D=48, E=24, F=16, G=16, H=24, I=38, J=20
    let widths = [14.0, 20.0, 16.0, 48.0, 24.0, 16.0, 16.0, 24.0, 38.0, 20.0];
    sheet.set_column_widths(&widths)?;

    // Title & Subtitle
    sheet.write_title_block(
        9,
        "审核失败明细",
        "数据源：已上传家电电脑.xlsx、已上传数码.xlsx、发票明细.xlsx；家电电脑按商品名称升序；发票金额单位：元",
    )?;

    // 1. Appliance Section
    sheet.write_section_header(8, "家电电脑审核失败明细")?;

    let app_headers = [
        "交易日期",
        "检索参考号",
        "状态",
        "描述",
        "发票号码",
        "发票金额",
        "购买方名称",
        "S/N码",
        "商品名称",
    ];
    sheet.write_headers(&app_headers)?;

    let failed_fields = [
        "交易日期",
        "检索参考号",
        "描述",
        "发票号码",
        "发票金额",
        "购买方名称",
        "S/N码",
    ];
    let app_columns = if app_failed.is_empty() {
        Vec::new()
    } else {
        app_up.select(failed_fields, "已上传家电电脑.xlsx")?
    };
    let dig_columns = if dig_failed.is_empty() {
        Vec::new()
    } else {
        dig_up.select(failed_fields, "已上传数码.xlsx")?
    };
    let write_fail_row = |ws: &mut Worksheet,
                          row: &[Data],
                          columns: &[ReportColumn],
                          extra1: &str,
                          extra2: Option<&str>,
                          cur_row: u32|
     -> Result<(), ReportError> {
        ws.set_row_height(cur_row, 22.0)?;
        for (column, target) in columns.iter().zip([0, 1, 3, 4, 5, 6, 7]) {
            if target == 3 {
                ws.write_string_with_format(cur_row, 2, "审核失败", &s.text_left)?;
            }
            column
                .kind
                .write(ws, s, cur_row, target, column.cell(row))?;
        }
        ws.write_string_with_format(cur_row, 8, extra1, &s.text_left)?;
        if let Some(e2) = extra2 {
            ws.write_string_with_format(cur_row, 9, e2, &s.text_left)?;
        }
        Ok(())
    };

    for record in app_failed {
        write_fail_row(
            sheet.ws,
            record.row,
            &app_columns,
            &record.extra1,
            None,
            sheet.cur_row,
        )?;
        sheet.cur_row += 1;
    }

    // Blank row
    sheet.write_blank_row(8.0)?;

    // 2. Digital Section
    sheet.write_section_header(9, "数码审核失败明细")?;

    let dig_headers = [
        "交易日期",
        "检索参考号",
        "状态",
        "描述",
        "发票号码",
        "发票金额",
        "购买方名称",
        "S/N码",
        "IMEI1",
        "IMEI2",
    ];
    sheet.write_headers(&dig_headers)?;

    for record in dig_failed {
        write_fail_row(
            sheet.ws,
            record.row,
            &dig_columns,
            &record.extra1,
            record.extra2.as_deref(),
            sheet.cur_row,
        )?;
        sheet.cur_row += 1;
    }

    Ok(())
}

fn build_refund_anomaly_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    app_refund_rows: &SheetData,
    dig_refund_rows: &SheetData,
    app_refund: &RefundAnomalySection<'_>,
    dig_refund: &RefundAnomalySection<'_>,
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "异常回款明细", s)?;

    // 24 column widths
    let widths = [
        32.0, 20.0, 16.0, 26.0, 26.0, 22.0, 18.0, 14.0, 14.0, 14.0, 14.0, 12.0, 24.0, 14.0, 16.0,
        12.0, 16.0, 36.0, 14.0, 24.0, 18.0, 24.0, 48.0, 18.0,
    ];
    sheet.set_column_widths(&widths)?;

    // Title & Subtitle
    sheet.write_title_block(
        23,
        "异常回款明细",
        "提取规则：筛选本店核销商编且单元格为粉色填充（颜色 #FFC7CE）；重复交易参考号在前，不重复记录在后并标黄；组内升序；金额单位：元",
    )?;

    let write_section = |sheet: &mut ReportSheetBuilder<'_>,
                         title: &str,
                         rows: &SheetData,
                         section: &RefundAnomalySection<'_>|
     -> Result<(), ReportError> {
        sheet.write_section_header(23, title)?;

        // Write header row (24 columns from source)
        let fields: Vec<String> = (0..24)
            .map(|col| rows[0].get(col).map(cell_to_string).unwrap_or_default())
            .collect();
        let field_strs: Vec<&str> = fields.iter().map(|f| f.as_str()).collect();
        let columns: Vec<ReportColumn> = fields
            .iter()
            .enumerate()
            .map(|(col, field)| ReportColumn::new(Some(col), field))
            .collect();
        sheet.write_headers(&field_strs)?;

        let data_start_row = sheet.cur_row;
        for row in &section.rows {
            sheet.write_row(row, &columns)?;
        }

        if data_start_row < sheet.cur_row {
            let reference_col = column_number_to_name(section.ref_col_idx as u16);
            let first_row = data_start_row + 1;
            let last_row = sheet.cur_row;
            let rule = format!(
                "=AND(${reference_col}{first_row}<>\"\",COUNTIF(${reference_col}${first_row}:${reference_col}${last_row},${reference_col}{first_row})=1)"
            );
            let highlight = ConditionalFormatFormula::new()
                .set_rule(rule.as_str())
                .set_format(Format::new().set_background_color(Color::RGB(0xFFF2CC)));
            sheet.ws.add_conditional_format(
                data_start_row,
                0,
                sheet.cur_row - 1,
                23,
                &highlight,
            )?;
        }
        Ok(())
    };

    write_section(
        &mut sheet,
        "家电电脑异常回款明细",
        app_refund_rows,
        app_refund,
    )?;

    // Blank row
    sheet.write_blank_row(8.0)?;

    write_section(&mut sheet, "数码异常回款明细", dig_refund_rows, dig_refund)?;

    Ok(())
}

fn build_invoice_anomaly_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    invoices: &SheetData,
    section: &InvoiceAnomalySection<'_>,
) -> Result<(), ReportError> {
    if invoices[0].len() < 8 {
        return Err("发票明细.xlsx: 表头不足 8 列".to_string().into());
    }

    let mut sheet = ReportSheetBuilder::new(wb, "异常发票明细", s)?;

    // 8 columns: widths [20.0, 14.0, 24.0, 18.0, 40.0, 58.0, 16.0, 22.0]
    let widths = [20.0, 14.0, 24.0, 18.0, 40.0, 58.0, 16.0, 22.0];
    sheet.set_column_widths(&widths)?;

    // Title & Subtitle
    sheet.write_title_block(
        7,
        "异常发票明细",
        "提取规则：发票明细黄色填充区域（颜色 #FFEB9C）；按匹配单据号升序排列",
    )?;

    // Section Header
    sheet.write_section_header(7, "黄色异常发票明细")?;

    let header_strings: Vec<String> = invoices[0].iter().take(8).map(cell_to_string).collect();
    let header_strs: Vec<&str> = header_strings.iter().map(|s| s.as_str()).collect();
    sheet.write_headers(&header_strs)?;

    let columns: Vec<_> = header_strings
        .iter()
        .enumerate()
        .map(|(col, field)| ReportColumn::new(Some(col), field))
        .collect();

    for row in &section.rows {
        sheet.write_row(row, &columns)?;
    }

    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn category_sheet_xml(path: &Path) -> String {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut xml = String::new();
        zip.by_name("xl/worksheets/sheet2.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn standalone_summary_generates_only_excel() {
        use crate::jobs::{refund::OUTPUT_FIELDS, uploaded};
        use calamine::{Reader, open_workbook_auto};
        let root = crate::test_support::unique_temp_path("summary-excel-only");
        let input = root.join("input");
        let output = root.join("output");
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir(&output).unwrap();
        let mut fixtures = vec![
            (
                "销售用券情况统计.xlsx",
                vec!["财务大类", "品牌", "补贴额", "数量", "备注"],
            ),
            (
                "发票明细.xlsx",
                vec![
                    "开票时间",
                    "开票类型",
                    "数电发票号码",
                    "购方名称",
                    "主要商品名称",
                    "备注信息",
                    "开票状态",
                    "匹配单据号",
                ],
            ),
            ("回款明细家电电脑.xlsx", OUTPUT_FIELDS.to_vec()),
            ("回款明细数码.xlsx", OUTPUT_FIELDS.to_vec()),
        ];
        for (name, tail) in [
            ("已上传家电电脑.xlsx", &uploaded::APPLIANCE_TAIL),
            ("已上传数码.xlsx", &uploaded::DIGITAL_TAIL),
        ] {
            fixtures.push((
                name,
                uploaded::FRONT_HEADERS
                    .iter()
                    .copied()
                    .chain(tail.iter().map(|field| field.synonyms[0]))
                    .chain(["补贴金额"])
                    .collect(),
            ));
        }
        for (name, headers) in fixtures {
            let mut book = Workbook::new();
            let sheet = book.add_worksheet();
            for (col, header) in headers.iter().enumerate() {
                sheet.write_string(0, col as u16, *header).unwrap();
            }
            if name.starts_with("已上传") {
                let merchant = headers.iter().position(|field| *field == "商户号").unwrap();
                let status = headers.iter().position(|field| *field == "状态").unwrap();
                sheet
                    .write_string(1, merchant as u16, "89813015722APT1")
                    .unwrap();
                sheet.write_string(1, status as u16, "审核终止").unwrap();
            }
            if name == "销售用券情况统计.xlsx" {
                for (index, (category, brand)) in [
                    ("冰箱", "海尔"),
                    ("冰箱", "海信"),
                    ("冰箱", "博世"),
                    ("彩电", "海信"),
                    ("彩电", "华为（终端）"),
                    ("彩电", "创维"),
                ]
                .into_iter()
                .enumerate()
                {
                    let row = index as u32 + 1;
                    sheet.write_string(row, 0, category).unwrap();
                    sheet.write_string(row, 1, brand).unwrap();
                    sheet.write_number(row, 2, 0.0).unwrap();
                    sheet.write_number(row, 3, 0.0).unwrap();
                    sheet.write_string(row, 4, "未上传").unwrap();
                }
            }
            book.save(input.join(name)).unwrap();
        }
        let path = output.join("summary.xlsx");
        generate_summary_workbook(&input, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        assert_eq!(book.sheet_names().len(), 5);
        let summary = book.worksheet_range("汇总").unwrap();
        assert_eq!(summary.get_value((18, 6)), Some(&Data::Float(0.0)));
        assert_eq!(summary.get_value((12, 2)), Some(&Data::Float(0.0)));
        assert_eq!(summary.get_value((12, 4)), Some(&Data::Float(0.0)));
        let failed = book.worksheet_range("审核失败明细").unwrap();
        assert!(
            !failed
                .rows()
                .flatten()
                .any(|cell| cell_to_string(cell) == "审核终止")
        );
        let brands = book.worksheet_range("品类品牌汇总").unwrap();
        let names: Vec<_> = brands
            .rows()
            .skip(5)
            .map(|row| cell_to_string(&row[1]))
            .collect();
        assert_eq!(
            names,
            ["博世", "海信", "海尔", "创维", "华为（终端）", "海信"]
        );
        assert_eq!(brands.get_value((5, 10)), Some(&Data::String("-".into())));
        assert_eq!(brands.get_value((5, 11)), Some(&Data::String("-".into())));
        {
            let xml = category_sheet_xml(&path);
            assert!(xml.contains("<mergeCell ref=\"A6:A8\"/>"));
            assert!(xml.contains("<mergeCell ref=\"A9:A11\"/>"));
        }

        // Verify presentation through the same standalone workbook interface.
        let mut source = Workbook::new();
        let sheet = source.add_worksheet();
        for (col, header) in ["财务大类", "品牌", "补贴额", "数量", "备注"]
            .iter()
            .enumerate()
        {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (index, (category, brand, amount, quantity, status)) in [
            ("Z", "Negative", -5.0, -1.0, "未上传"),
            ("A", "ZeroQty", 4.0, 0.0, "未上传"),
            ("A", "ZeroAmount", 0.0, 2.0, "未上传"),
            ("A", "NoneState", 100.0, 1.0, "其他"),
            ("A", "Zero", 0.0, 0.0, "未上传"),
            ("A", "Return", 100.0, 1.0, "退货-原单"),
            ("", "", 0.0, 0.0, ""),
        ]
        .into_iter()
        .enumerate()
        {
            let row = index as u32 + 1;
            sheet.write_string(row, 0, category).unwrap();
            sheet.write_string(row, 1, brand).unwrap();
            sheet.write_number(row, 2, amount).unwrap();
            sheet.write_number(row, 3, quantity).unwrap();
            sheet.write_string(row, 4, status).unwrap();
        }
        source.save(input.join("销售用券情况统计.xlsx")).unwrap();
        generate_summary_workbook(&input, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        let brands = book.worksheet_range("品类品牌汇总").unwrap();
        let names: Vec<_> = brands
            .rows()
            .skip(5)
            .map(|row| cell_to_string(&row[1]))
            .collect();
        assert_eq!(
            names,
            ["", "NoneState", "Zero", "ZeroAmount", "ZeroQty", "Negative"]
        );
        assert_eq!(brands.get_value((6, 0)), Some(&Data::String("A".into())));
        assert_eq!(brands.get_value((10, 0)), Some(&Data::String("Z".into())));
        for row in 5..=7 {
            for col in 2..12 {
                assert_eq!(
                    brands.get_value((row, col)),
                    Some(&Data::String("-".into()))
                );
            }
        }
        for (row, amount, quantity) in [(8, 0.0, 2.0), (9, 4.0, 0.0), (10, -5.0, -1.0)] {
            assert_eq!(brands.get_value((row, 10)), Some(&Data::Float(amount)));
            assert_eq!(brands.get_value((row, 11)), Some(&Data::Float(quantity)));
        }
        {
            let xml = category_sheet_xml(&path);
            assert!(xml.contains("<mergeCell ref=\"A7:A10\"/>"));
            assert!(!xml.contains("<mergeCell ref=\"A11:"));
        }
        assert!(!path.with_extension("md").exists());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminated_uploads_are_excluded_from_metrics_and_failure_records() {
        let root = std::env::temp_dir().join(format!("subsidy-status-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("uploads.xlsx");
        let output = root.join("failures.xlsx");
        let headers = [
            "备注",
            "补贴金额",
            "状态",
            "检索参考号",
            "发票号码",
            "交易日期",
            "描述",
            "发票金额",
            "购买方名称",
            "S/N码",
            "IMEI1",
            "IMEI2",
            "实时清分UUID",
            "更新时间",
        ];
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        for (col, header) in headers.iter().enumerate() {
            ws.write_string(0, col as u16, *header).unwrap();
        }
        for (index, (status, amount)) in [
            (" 审核终止 ", "20.55"),
            ("审核失败", "10"),
            ("已回款", "5"),
            ("", "0"),
        ]
        .iter()
        .enumerate()
        {
            let row = index as u32 + 1;
            ws.write_string(row, 0, "审核终止").unwrap();
            ws.write_string(row, 1, *amount).unwrap();
            ws.write_string(row, 2, *status).unwrap();
            ws.write_string(row, 3, format!("ref-{index}")).unwrap();
            ws.write_string(row, 5, "2026年03月30日").unwrap();
        }
        wb.save(&source).unwrap();
        let original = std::fs::read(&source).unwrap();
        let uploads = SheetData::load(&source).unwrap();
        assert_eq!(cell_to_string(&uploads[1][2]), "审核终止");
        assert_eq!(cell_to_string(&uploads[1][0]), "审核终止");
        assert_eq!(cell_to_string(&uploads[3][2]), "已回款");
        assert_eq!(cell_to_string(&uploads[4][2]), "");
        let sales = vec![
            vec![
                Data::String("财务大类".into()),
                Data::String("补贴额".into()),
                Data::String("数量".into()),
            ],
            vec![Data::String("家电".into()), Data::Int(100), Data::Int(5)],
        ];
        let metrics =
            SummaryMetrics::calculate(&SheetData::new(sales), &uploads, &uploads).unwrap();
        assert_eq!(metrics.fail.app_amt, Decimal::new(1000, 2));
        assert_eq!(metrics.fail.app_cnt, 1);
        assert_eq!(metrics.fail.dig_amt, Decimal::new(1000, 2));
        assert_eq!(metrics.fail.dig_cnt, 1);
        assert_eq!(metrics.unup.app_amt, Decimal::new(8500, 2));
        assert_eq!(metrics.unup.app_cnt, 3);
        let mut failures = Workbook::new();
        let empty_names = UniqueIndex::default();
        let app_failed = FailedUploadRecord::extract_appliance(&uploads, &empty_names).unwrap();
        let dig_failed = FailedUploadRecord::extract_digital(&uploads).unwrap();
        build_failed_records_sheet(
            &mut failures,
            &StylePool::default(),
            &uploads,
            &uploads,
            &app_failed,
            &dig_failed,
        )
        .unwrap();
        failures.save(&output).unwrap();
        let rows = read_sheet_rows(&output).unwrap();
        let refs: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.get(2)
                    .is_some_and(|cell| cell_to_string(cell) == "审核失败")
            })
            .map(|row| cell_to_string(&row[1]))
            .collect();
        assert_eq!(refs, ["ref-1", "ref-1"]);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(
            cell_to_string(&read_sheet_rows(&source).unwrap()[1][2]),
            "审核终止"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
