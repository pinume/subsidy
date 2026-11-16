use rust_decimal::Decimal;
use rust_xlsxwriter::{Color, ConditionalFormatFormula, Format, Workbook, column_number_to_name};
use std::path::Path;

use super::excel::{ReportError, ReportSheetBuilder};
use super::reader::{get_colored_row_indices, get_colored_row_indices_multi};
use super::styles::StylePool;

use super::data::{CommonInputs, ReportColumn};
use super::summary_data::{
    AnomalyColors, AnomalyDatasets, BrandOccurrence, CategoryBrandSummary, DetailSection,
    OCCURRENCE_METRICS, SUMMARY_STATUSES, SummaryMetrics,
};

/// Generate Workbook 1: 国补上传情况汇总.xlsx (6 Sheets)
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

    let refund_pink = |name: &str| get_colored_row_indices(&input_dir.join(name), "FFFFC7CE");
    let mut inv_colors =
        get_colored_row_indices_multi(&input_dir.join("发票明细.xlsx"), &["FFFFEB9C", "FFFFC7CE"])?;
    let colors = AnomalyColors {
        app_pink: refund_pink("回款明细家电电脑.xlsx")?,
        dig_pink: refund_pink("回款明细数码.xlsx")?,
        invoice_yellow: inv_colors.remove("FFFFEB9C").unwrap_or_default(),
        invoice_pink: inv_colors.remove("FFFFC7CE").unwrap_or_default(),
    };

    let metrics = SummaryMetrics::calculate(&inputs.sales, &inputs.app_upload, &inputs.dig_upload)?;
    let category_brands = CategoryBrandSummary::from_sales(&inputs.sales)?;
    let occurrence = BrandOccurrence::from_sales(&inputs.sales)?;
    let datasets = AnomalyDatasets::extract(inputs, &colors)?;

    build_summary_sheet(&mut workbook, &styles, &metrics)?;
    build_category_brand_sheet(&mut workbook, &styles, &category_brands)?;
    build_failed_records_sheet(&mut workbook, &styles, datasets.failed())?;
    build_refund_anomaly_sheet(&mut workbook, &styles, datasets.refunds())?;
    build_invoice_anomaly_sheet(&mut workbook, &styles, datasets.invoices())?;
    build_occurrence_sheet(&mut workbook, &styles, &occurrence)?;

    workbook
        .save(output_path)
        .map_err(|e| format!("保存工作簿一失败: {}", e))?;

    Ok([metrics.duplicate_warning(), datasets.warning()]
        .into_iter()
        .flatten()
        .collect())
}

/// 按区块表头写列头，再逐行写入；列类型由表头字段名决定。
fn write_detail_section(
    sheet: &mut ReportSheetBuilder<'_>,
    section: &DetailSection,
) -> Result<(), ReportError> {
    let headers: Vec<&str> = section.headers().iter().map(String::as_str).collect();
    sheet.write_headers(&headers)?;
    let columns: Vec<_> = headers
        .iter()
        .enumerate()
        .map(|(col, field)| ReportColumn::new(Some(col), field))
        .collect();
    for row in section.rows() {
        sheet.write_row(row, &columns)?;
    }
    Ok(())
}

/// 两层表头：首列纵向合并，其后每个分组横向合并并在第二行写入子列名。
fn write_grouped_header(
    sheet: &mut ReportSheetBuilder<'_>,
    leading: &[&str],
    groups: &[&str],
    sub: &[&str],
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
        let col = (leading.len() + index * sub.len()) as u16;
        let last = col + sub.len() as u16 - 1;
        sheet
            .ws
            .merge_range(top, col, top, last, group, &s.col_header_center)?;
        for (offset, label) in sub.iter().enumerate() {
            sheet.ws.write_string_with_format(
                top + 1,
                col + offset as u16,
                *label,
                &s.col_header_center,
            )?;
        }
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
    write_grouped_header(&mut sheet, &["项目"], &groups, &["金额（万元）", "笔数"])?;

    // Convert Yuan to Wan Yuan
    let to_wan = |d: Decimal| d / Decimal::from(10000);

    let (overall, unpaid_detail) = m.rows();
    for (part, rows) in [&overall[..], &unpaid_detail[..]].into_iter().enumerate() {
        if part == 1 {
            write_tip_row(&mut sheet)?;
        }
        for (name, row) in rows {
            let (r, ws) = (sheet.cur_row, &mut *sheet.ws);
            ws.set_row_height(r, 22.0)?;
            ws.write_string_with_format(r, 0, *name, &s.text_left)?;
            for (i, (amount, count)) in row.by_class().into_iter().enumerate() {
                let col = 1 + 2 * i as u16;
                ws.write_with_format(r, col, to_wan(amount), &s.money)?;
                ws.write_number_with_format(r, col + 1, count as f64, &s.int_count)?;
            }
            sheet.cur_row += 1;
        }
    }

    sheet
        .write_blank_row(8.0)?
        .write_section_header(6, "二、结构与比率分析")?;
    write_grouped_header(
        &mut sheet,
        &["指标"],
        &groups,
        &["金额（万元）", "占比/比率"],
    )?;

    let (overall, unpaid_detail) = m.amount_ratios();
    for (part, rows) in [&overall[..], &unpaid_detail[..]].into_iter().enumerate() {
        if part == 1 {
            write_tip_row(&mut sheet)?;
        }
        for (name, values) in rows {
            let (r, ws) = (sheet.cur_row, &mut *sheet.ws);
            ws.set_row_height(r, 24.0)?;
            ws.write_string_with_format(r, 0, *name, &s.text_left)?;
            for (i, (amount, ratio)) in values.iter().enumerate() {
                let col = 1 + 2 * i as u16;
                ws.write_with_format(r, col, to_wan(*amount), &s.money)?;
                ws.write_number_with_format(r, col + 1, *ratio, &s.percent)?;
            }
            sheet.cur_row += 1;
        }
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
        &["补贴金额（元）", "数量"],
    )?;

    let ws = &mut *sheet.ws;
    let mut row_idx = sheet.cur_row;
    for group in summary.categories() {
        let cat = group.category();
        let start_row = row_idx;
        for brand in group.brands() {
            ws.set_row_height(row_idx, 22.0)?;
            ws.write_string_with_format(row_idx, 0, cat, &s.text_center)?;
            ws.write_string_with_format(row_idx, 1, brand.brand(), &s.text_left)?;

            for (s_idx, (amt, cnt)) in brand.statuses().enumerate() {
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
    [appliance, digital]: &[DetailSection; 2],
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "审核失败明细", s)?;
    let widths = [14.0, 20.0, 16.0, 48.0, 24.0, 16.0, 16.0, 24.0, 38.0, 20.0];
    sheet.set_column_widths(&widths)?.write_title_block(
        9,
        "审核失败明细",
        "数据源：已上传家电电脑.xlsx、已上传数码.xlsx、发票明细.xlsx；家电电脑按商品名称升序；发票金额单位：元",
    )?;
    sheet.write_section_header(8, "家电电脑审核失败明细")?;
    write_detail_section(&mut sheet, appliance)?;
    sheet.write_blank_row(8.0)?;
    sheet.write_section_header(9, "数码审核失败明细")?;
    write_detail_section(&mut sheet, digital)
}

fn build_refund_anomaly_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    refunds: &[DetailSection; 2],
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "异常回款明细", s)?;
    let widths = [
        32.0, 20.0, 16.0, 26.0, 26.0, 22.0, 18.0, 14.0, 14.0, 14.0, 14.0, 12.0, 24.0, 14.0, 16.0,
        12.0, 16.0, 36.0, 14.0, 24.0, 18.0, 24.0, 48.0, 18.0,
    ];
    sheet.set_column_widths(&widths)?.write_title_block(
        23,
        "异常回款明细",
        "提取规则：筛选本店核销商编且单元格为粉色填充（颜色 #FFC7CE）；重复交易参考号在前，不重复记录在后并标黄；组内升序；金额单位：元",
    )?;

    for (index, (title, section)) in ["家电电脑异常回款明细", "数码异常回款明细"]
        .into_iter()
        .zip(refunds)
        .enumerate()
    {
        if index > 0 {
            sheet.write_blank_row(8.0)?;
        }
        sheet.write_section_header(23, title)?;
        let data_start_row = sheet.cur_row + 1;
        write_detail_section(&mut sheet, section)?;
        if let Some(col) = section.unique_highlight_col()
            && data_start_row < sheet.cur_row
        {
            let reference_col = column_number_to_name(col);
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
    }
    Ok(())
}

fn build_invoice_anomaly_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    section: &DetailSection,
) -> Result<(), ReportError> {
    let mut sheet = ReportSheetBuilder::new(wb, "异常发票明细", s)?;
    let widths = [20.0, 14.0, 24.0, 18.0, 40.0, 58.0, 16.0, 22.0];
    sheet.set_column_widths(&widths)?.write_title_block(
        7,
        "异常发票明细",
        "提取规则：发票明细黄色填充区域（颜色 #FFEB9C）；按匹配单据号升序排列",
    )?;
    sheet.write_section_header(7, "黄色异常发票明细")?;
    write_detail_section(&mut sheet, section)
}

/// 报表规则第4.6节：固定品牌组横向展开三项累计金额，单行数据。
fn build_occurrence_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    occurrence: &BrandOccurrence,
) -> Result<(), ReportError> {
    let labels: Vec<_> = occurrence.brand_groups().map(|(label, _)| label).collect();
    let width = labels.len() * OCCURRENCE_METRICS.len();
    let mut sheet = ReportSheetBuilder::new(wb, "国补发生情况", s)?;
    sheet
        .set_column_widths(&vec![16.0; width])?
        .write_title_block(
            width as u16 - 1,
            "国补发生情况",
            "数据源：销售用券情况统计.xlsx；按全量数据累计，含退货正负金额；金额单位：元",
        )?;
    write_grouped_header(&mut sheet, &[], &labels, &OCCURRENCE_METRICS)?;
    let r = sheet.cur_row;
    sheet.ws.set_row_height(r, 22.0)?;
    for (index, (_, values)) in occurrence.brand_groups().enumerate() {
        for (offset, value) in values.into_iter().enumerate() {
            let col = (index * OCCURRENCE_METRICS.len() + offset) as u16;
            sheet.ws.write_with_format(r, col, value, &s.money)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::reporting::data::SheetData;
    use crate::reporting::reader::{cell_to_string, read_sheet_rows};
    use crate::reporting::unique_index::UniqueIndex;
    use calamine::Data;

    fn zip_entry(path: &Path, name: &str) -> String {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut xml = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
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
        assert_eq!(
            book.sheet_names(),
            [
                "汇总",
                "品类品牌汇总",
                "审核失败明细",
                "异常回款明细",
                "异常发票明细",
                "国补发生情况"
            ]
        );
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
            let xml = zip_entry(&path, "xl/worksheets/sheet2.xml");
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
            let xml = zip_entry(&path, "xl/worksheets/sheet2.xml");
            assert!(xml.contains("<mergeCell ref=\"A7:A10\"/>"));
            assert!(!xml.contains("<mergeCell ref=\"A11:"));
        }

        // 报表规则第4.6节：国补发生情况读取生成的 XLSX 验证结构与金额落位；
        // 归组、品类限制与退货口径由 summary_data 的单元测试覆盖。
        let mut source = Workbook::new();
        let sheet = source.add_worksheet();
        for (col, header) in ["财务大类", "品牌", "补贴额", "数量", "备注"]
            .iter()
            .enumerate()
        {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (index, (category, brand, amount, status)) in [
            ("冰箱", "海尔", 100.5, "已回款"),
            ("厨卫", "AO史密斯", -40.0, "未上传"),
        ]
        .into_iter()
        .enumerate()
        {
            let row = index as u32 + 1;
            sheet.write_string(row, 0, category).unwrap();
            sheet.write_string(row, 1, brand).unwrap();
            sheet.write_number(row, 2, amount).unwrap();
            sheet.write_number(row, 3, 1.0).unwrap();
            sheet.write_string(row, 4, status).unwrap();
        }
        source.save(input.join("销售用券情况统计.xlsx")).unwrap();
        generate_summary_workbook(&input, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        assert_eq!(book.sheet_names()[5], "国补发生情况");
        let occurrence = book.worksheet_range("国补发生情况").unwrap();
        assert_eq!(occurrence.end(), Some((5, 38)));
        let groups: Vec<_> = (0..13)
            .map(|i| cell_to_string(occurrence.get_value((3, i * 3)).unwrap()))
            .collect();
        assert_eq!(
            groups,
            [
                "海尔系",
                "美的系",
                "海信系",
                "TCL系",
                "博世系",
                "西门子系",
                "老板",
                "方太",
                "史密斯",
                "格力空调",
                "创维电视",
                "华为（数码+新业务）",
                "小米（新业务）"
            ]
        );
        for col in 0..39 {
            let metric = ["使用金额", "已回款", "未回款"][col as usize % 3];
            assert_eq!(
                cell_to_string(occurrence.get_value((4, col)).unwrap()),
                metric
            );
        }
        // 海尔系在 A–C，无数据的美的系为 0，史密斯（第9组）为负数。
        for (col, value) in [
            (0, 100.5),
            (1, 100.5),
            (2, 0.0),
            (3, 0.0),
            (24, -40.0),
            (25, 0.0),
            (26, -40.0),
        ] {
            assert_eq!(
                occurrence.get_value((5, col)),
                Some(&Data::Float(value)),
                "col {col}"
            );
        }
        let xml = zip_entry(&path, "xl/worksheets/sheet6.xml");
        for merge in ["A1:AM1", "A2:AM2", "A4:C4", "AK4:AM4"] {
            assert!(
                xml.contains(&format!("<mergeCell ref=\"{merge}\"/>")),
                "{merge}"
            );
        }
        assert!(xml.contains("<cols><col min=\"1\" max=\"39\" width=\"16.7109375\""));
        // 第3行留白。
        assert!((0..39).all(|col| occurrence.get_value((2, col)) == Some(&Data::Empty)));
        // 行高 30/22/8/30/30/22；写出时按像素取整，22→21.75、8→8.25。
        for (row, height) in [
            (1, "30"),
            (2, "21.75"),
            (3, "8.25"),
            (4, "30"),
            (5, "30"),
            (6, "21.75"),
        ] {
            assert!(
                xml.contains(&format!("<row r=\"{row}\" spans=\"1:39\" ht=\"{height}\"")),
                "row {row}"
            );
        }
        let styles = zip_entry(&path, "xl/styles.xml");
        // 金额行全部使用两位小数、负数红字的金额格式。
        let money_id = styles
            .split("<numFmt ")
            .find(|f| f.contains("formatCode=\"#,##0.00;[Red]-#,##0.00\""))
            .and_then(|f| f.split('"').nth(1))
            .unwrap();
        let xfs: Vec<_> = styles
            .split("<cellXfs")
            .nth(1)
            .unwrap()
            .split("<xf ")
            .skip(1)
            .collect();
        let row6 = xml
            .split("<row r=\"6\"")
            .nth(1)
            .unwrap()
            .split("</row>")
            .next()
            .unwrap();
        assert_eq!(row6.matches("<c ").count(), 39);
        for cell in row6.split("<c ").skip(1) {
            let style: usize = cell
                .split("s=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!(xfs[style].contains(&format!("numFmtId=\"{money_id}\"")));
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
        let (_, [.., (_, fail), (_, unup)]) = metrics.rows();
        let [app, dig, _] = fail.by_class();
        assert_eq!(app, (Decimal::new(1000, 2), 1));
        assert_eq!(dig, (Decimal::new(1000, 2), 1));
        assert_eq!(unup.by_class()[0], (Decimal::new(8500, 2), 3));
        let mut failures = Workbook::new();
        let empty_names = UniqueIndex::default();
        let sections = [
            DetailSection::failed_appliance(&uploads, &empty_names).unwrap(),
            DetailSection::failed_digital(&uploads).unwrap(),
        ];
        build_failed_records_sheet(&mut failures, &StylePool::default(), &sections).unwrap();
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
