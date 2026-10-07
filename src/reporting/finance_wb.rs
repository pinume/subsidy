use calamine::Data;
use rust_xlsxwriter::{Workbook, Worksheet};
use std::collections::HashMap;
use std::path::Path;

use super::data::{CellKind, CommonInputs, ReportColumn, SheetData};
use super::reader::{HeaderMap, UploadColumns, cell_to_string};
use super::styles::StylePool;

/// Generate Workbook 2: 26年国补门店财务统筹表.xlsx (4 Sheets)
pub fn generate_store_finance_workbook(input_dir: &Path, output_path: &Path) -> Result<(), String> {
    let inputs = CommonInputs::load(input_dir)?;
    generate_with_inputs(input_dir, output_path, &inputs)
}

pub(crate) fn generate_with_inputs(
    input_dir: &Path,
    output_path: &Path,
    inputs: &CommonInputs,
) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let styles = StylePool::default();

    // 1. Load source data
    let store_occ_rows = SheetData::load(&input_dir.join("银联交易明细门店.xlsx"), false)?;
    let sales_rows = &inputs.sales;
    let app_upload_rows = &inputs.app_upload;
    let dig_upload_rows = &inputs.dig_upload;
    let app_refund_rows = &inputs.app_refund;
    let dig_refund_rows = &inputs.dig_refund;
    let invoice_rows = &inputs.invoices;
    let app_store_code = &inputs.app_store_code;
    let dig_store_code = &inputs.dig_store_code;

    // Build Sheet 1: 最终匹配表（全部的国补发生数据上匹配）
    build_final_match_sheet(
        &mut workbook,
        &styles,
        &store_occ_rows,
        sales_rows,
        app_upload_rows,
        dig_upload_rows,
        invoice_rows,
    )?;

    // Build Sheet 2: 1.门店国补发生表（银联系统直接导出，不需要加工）
    build_store_occurrence_sheet(&mut workbook, &styles, &store_occ_rows)?;

    // Build Sheet 3: 2.门店累计回款表（事业部财务下发，门店筛选自己的）
    build_store_refund_sheet(
        &mut workbook,
        &styles,
        app_refund_rows,
        dig_refund_rows,
        app_store_code,
        dig_store_code,
    )?;

    // Build Sheet 4: 3.门店上传明细（从门店银联后台每月导出后汇总）
    build_store_upload_sheet(&mut workbook, &styles, app_upload_rows, dig_upload_rows)?;

    workbook
        .save(output_path)
        .map_err(|e| format!("保存工作簿二失败: {}", e))?;

    Ok(())
}

fn build_final_match_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    store_occ: &SheetData,
    sales: &SheetData,
    app_up: &SheetData,
    dig_up: &SheetData,
    invoices: &SheetData,
) -> Result<(), String> {
    let ws = wb.add_worksheet();
    ws.set_name("最终匹配表（全部的国补发生数据上匹配）")
        .map_err(|e| e.to_string())?;
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;

    // 27 columns widths
    let widths = [
        8.0, 20.0, 20.0, 14.0, 12.0, 22.0, 14.0, 14.0, 12.0, 16.0, 18.0, 12.0, 16.0, 18.0, 24.0,
        16.0, 28.0, 28.0, 14.0, 16.0, 14.0, 14.0, 16.0, 26.0, 16.0, 24.0, 14.0,
    ];
    for (col, w) in widths.iter().enumerate() {
        ws.set_column_width(col as u16, *w)
            .map_err(|e| e.to_string())?;
    }

    // Headers
    let headers = [
        "序号",
        "清算时间",
        "交易时间",
        "终端号",
        "交易类型",
        "卡号",
        "交易金额",
        "清算金额",
        "手续费",
        "流水号",
        "检索号",
        "卡类型",
        "发卡行",
        "商户号",
        "商户名称",
        "分店简称",
        "商户订单号",
        "银商订单号",
        "交易方式",
        "分店",
        "优惠金额",
        "大类",
        "品牌",
        "产品型号",
        "状态",
        "发票号",
        "发票是否红冲",
    ];

    ws.set_row_height(0, 30.0).map_err(|e| e.to_string())?;
    for (col, h) in headers.iter().enumerate() {
        let fmt = if col == 0 || col == 21 || col == 24 || col == 26 {
            &s.col_header_center
        } else {
            &s.col_header
        };
        ws.write_string_with_format(0, col as u16, *h, fmt)
            .map_err(|e| e.to_string())?;
    }

    // 1. Index sales products by invoice; the first source row wins.
    let sales_h = &sales.header;
    let sales_invoice_idx = sales_h.require("数电发票号码", "销售用券情况统计.xlsx")?;
    let sales_cat_idx = sales_h.require("财务大类", "销售用券情况统计.xlsx")?;
    let sales_brand_idx = sales_h.require("品牌", "销售用券情况统计.xlsx")?;
    let sales_name_idx = sales_h.require("商品名称", "销售用券情况统计.xlsx")?;
    let mut product_map = HashMap::new();
    for row in &sales[1..] {
        let invoice = cell_to_string(&row[sales_invoice_idx]);
        if !invoice.is_empty() {
            product_map.entry(invoice).or_insert(row);
        }
    }

    // 2. Build Index for 门店上传明细 (appliance + digital)
    let mut upload_map: HashMap<String, (String, String)> = HashMap::new();
    for (rows, label) in [(app_up, "已上传家电电脑.xlsx"), (dig_up, "已上传数码.xlsx")]
    {
        let cols = UploadColumns::from_header(&rows.header, label)?;
        for row in &rows[1..] {
            let r_no = cell_to_string(&row[cols.reference]);
            if !r_no.is_empty() {
                upload_map.insert(
                    r_no,
                    (
                        cell_to_string(&row[cols.status]),
                        cell_to_string(&row[cols.invoice]),
                    ),
                );
            }
        }
    }

    // 3. Build Index for 发票明细
    let inv_h = &invoices.header;
    let inv_no_idx = inv_h
        .find(&["数电发票号码"])
        .ok_or("发票明细缺少数电发票号码")?;
    let inv_type_idx = inv_h.find(&["开票类型"]).ok_or("发票明细缺少开票类型")?;
    let inv_st_idx = inv_h.find(&["开票状态"]).ok_or("发票明细缺少开票状态")?;

    let mut invoice_map: HashMap<String, (String, String)> = HashMap::new();
    for row in &invoices[1..] {
        let no = cell_to_string(&row[inv_no_idx]);
        if !no.is_empty() {
            invoice_map.insert(
                no,
                (
                    cell_to_string(&row[inv_type_idx]),
                    cell_to_string(&row[inv_st_idx]),
                ),
            );
        }
    }

    // 4. Pre-resolve store occurrence column mappings for final match
    let store_h = &store_occ.header;
    let s_ref_idx = store_h.find(&["检索号"]).ok_or("门店发生表缺少检索号")?;
    let s_rem_idx = store_h.find(&["备注"]).ok_or("门店发生表缺少备注")?;

    let resolved_store_cols =
        store_occ.select(headers[1..21].iter().copied(), "银联交易明细门店.xlsx")?;

    for (r_idx, row) in store_occ.iter().enumerate().skip(1) {
        let cur_row = r_idx as u32; // Row 1 is header, data starts at row 1
        ws.set_row_height(cur_row, 22.0)
            .map_err(|e| e.to_string())?;

        // Col 0: 序号 (1-based)
        ws.write_number_with_format(cur_row, 0, r_idx as f64, &s.int_count)
            .map_err(|e| e.to_string())?;

        // Col 1-20: Pre-resolved from store occurrence
        for (index, column) in resolved_store_cols.iter().enumerate() {
            column
                .kind
                .write(ws, s, cur_row, index as u16 + 1, column.cell(row))?;
        }

        let ref_num = cell_to_string(&row[s_ref_idx]);
        let up_info = upload_map.get(&ref_num);
        let invoice_no = match up_info {
            Some((_, inv)) if !inv.is_empty() => inv.as_str(),
            _ => "",
        };

        // Col 21, 22, 23: match products only by the final invoice number.
        let (cat, brand, model) = match product_map.get(invoice_no) {
            Some(r) => (
                cell_to_string(&r[sales_cat_idx]),
                cell_to_string(&r[sales_brand_idx]),
                cell_to_string(&r[sales_name_idx]),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        ws.write_string_with_format(cur_row, 21, cat, &s.text_center)
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(cur_row, 22, brand, &s.text_left)
            .map_err(|e| e.to_string())?;
        ws.write_string_with_format(cur_row, 23, model, &s.text_left)
            .map_err(|e| e.to_string())?;

        // Col 24: 状态
        let remark = cell_to_string(&row[s_rem_idx]);
        let status = if remark == "已退货" {
            "已退货"
        } else if let Some((st, _)) = up_info {
            st.as_str()
        } else {
            "未提交"
        };
        ws.write_string_with_format(cur_row, 24, status, &s.text_center)
            .map_err(|e| e.to_string())?;

        // Col 25: 发票号
        ws.write_string_with_format(cur_row, 25, invoice_no, &s.text_left)
            .map_err(|e| e.to_string())?;

        // Col 26: 发票是否红冲
        let red_flush = if invoice_no.is_empty() {
            ""
        } else if let Some((inv_type, inv_status)) = invoice_map.get(invoice_no) {
            if inv_type == "红票" || inv_status == "已红冲" {
                "是"
            } else if inv_type == "蓝票" && inv_status == "开票完成" {
                "否"
            } else {
                ""
            }
        } else {
            ""
        };
        ws.write_string_with_format(cur_row, 26, red_flush, &s.text_center)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn build_store_occurrence_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    store_occ: &SheetData,
) -> Result<(), String> {
    let ws = wb.add_worksheet();
    ws.set_name("1.门店国补发生表（银联系统直接导出，不需要加工）")
        .map_err(|e| e.to_string())?;
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;

    // 26 columns widths
    let widths = [
        20.0, 20.0, 14.0, 12.0, 22.0, 14.0, 14.0, 12.0, 12.0, 12.0, 16.0, 18.0, 12.0, 16.0, 18.0,
        24.0, 16.0, 28.0, 28.0, 14.0, 16.0, 14.0, 14.0, 14.0, 16.0, 18.0,
    ];
    for (col, w) in widths.iter().enumerate() {
        ws.set_column_width(col as u16, *w)
            .map_err(|e| e.to_string())?;
    }

    // Row 1: Header (26 columns straight from source)
    if store_occ[0].len() < 26 {
        return Err("银联交易明细门店.xlsx: 表头不足 26 列".to_string());
    }
    ws.set_row_height(0, 30.0).map_err(|e| e.to_string())?;
    for (col, cell) in store_occ[0].iter().take(26).enumerate() {
        let name = cell_to_string(cell);
        ws.write_string_with_format(0, col as u16, name, &s.col_header)
            .map_err(|e| e.to_string())?;
    }

    let columns: Vec<_> = store_occ[0]
        .iter()
        .take(26)
        .enumerate()
        .map(|(col, field)| ReportColumn::new(Some(col), &cell_to_string(field)))
        .collect();
    for (r_idx, row) in store_occ.iter().enumerate().skip(1) {
        let cur_row = r_idx as u32;
        ws.set_row_height(cur_row, 22.0)
            .map_err(|e| e.to_string())?;
        for (col, column) in columns.iter().enumerate() {
            column
                .kind
                .write(ws, s, cur_row, col as u16, column.cell(row))?;
        }
    }

    Ok(())
}

fn build_store_refund_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    app_refund: &SheetData,
    dig_refund: &SheetData,
    app_store_code: &str,
    dig_store_code: &str,
) -> Result<(), String> {
    let ws = wb.add_worksheet();
    ws.set_name("2.门店累计回款表（事业部财务下发，门店筛选自己的）")
        .map_err(|e| e.to_string())?;
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;

    let widths = [
        32.0, 20.0, 16.0, 26.0, 26.0, 22.0, 18.0, 14.0, 14.0, 14.0, 14.0, 12.0, 24.0, 14.0, 16.0,
        12.0, 16.0, 36.0, 14.0, 24.0, 18.0, 24.0, 48.0, 18.0,
    ];
    for (col, w) in widths.iter().enumerate() {
        ws.set_column_width(col as u16, *w)
            .map_err(|e| e.to_string())?;
    }

    // Header (24 columns from source)
    if app_refund[0].len() < 24 {
        return Err("回款明细家电电脑.xlsx: 表头不足 24 列".to_string());
    }
    let fields: Vec<_> = app_refund[0].iter().take(24).map(cell_to_string).collect();
    ws.set_row_height(0, 30.0).map_err(|e| e.to_string())?;
    for (col, field) in fields.iter().enumerate() {
        ws.write_string_with_format(0, col as u16, field, &s.col_header)
            .map_err(|e| e.to_string())?;
    }

    let mut cur_row: u32 = 1;

    let append_rows = |ws: &mut Worksheet,
                       rows: &SheetData,
                       store_code: &str,
                       cur_row: &mut u32|
     -> Result<(), String> {
        let h = &rows.header;
        let mch_idx = h.find(&["核销商编"]).ok_or("回款明细缺少核销商编")?;
        let columns = rows.select(fields.iter().map(String::as_str), "回款明细")?;

        for row in &rows[1..] {
            if cell_to_string(&row[mch_idx]) == store_code {
                ws.set_row_height(*cur_row, 22.0)
                    .map_err(|e| e.to_string())?;
                for (col, column) in columns.iter().enumerate() {
                    column
                        .kind
                        .write(ws, s, *cur_row, col as u16, column.cell(row))?;
                }
                *cur_row += 1;
            }
        }
        Ok(())
    };

    append_rows(ws, app_refund, app_store_code, &mut cur_row)?;
    append_rows(ws, dig_refund, dig_store_code, &mut cur_row)?;

    Ok(())
}

fn build_store_upload_sheet(
    wb: &mut Workbook,
    s: &StylePool,
    app_up: &SheetData,
    dig_up: &SheetData,
) -> Result<(), String> {
    let ws = wb.add_worksheet();
    ws.set_name("3.门店上传明细（从门店银联后台每月导出后汇总）")
        .map_err(|e| e.to_string())?;
    ws.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;

    // 60 column headers and widths as defined in Section 5.4.2
    let col_defs: [(&str, f64); 60] = [
        ("实时清分UUID", 28.0),
        ("商户号", 18.0),
        ("商户名称", 24.0),
        ("订单号", 28.0),
        ("交易日期", 14.0),
        ("交易金额", 14.0),
        ("检索参考号", 18.0),
        ("模版类型", 12.0),
        ("状态", 14.0),
        ("描述", 32.0),
        ("提交时间", 20.0),
        ("更新时间", 20.0),
        ("终端号", 14.0),
        ("分店id", 26.0),
        ("分店名", 18.0),
        ("所在地区", 20.0),
        ("详细地址", 30.0),
        ("地区编码", 12.0),
        ("tel", 16.0),
        ("发票号码", 24.0),
        ("发票金额", 14.0),
        ("购买方名称", 16.0),
        ("图片1", 26.0),
        ("S/N码", 24.0),
        ("是否属于 AI 产品", 16.0),
        ("IMEI1", 20.0),
        ("IMEI2", 20.0),
        ("图片1", 26.0),
        ("图片2", 26.0),
        ("图片3", 26.0),
        ("图片4", 26.0),
        ("img5", 26.0),
        ("img6", 26.0),
        ("img7", 26.0),
        ("img8", 26.0),
        ("img9", 26.0),
        ("img10", 26.0),
        ("img11", 26.0),
        ("img12", 26.0),
        ("img13", 26.0),
        ("img14", 26.0),
        ("img15", 26.0),
        ("签收时间", 20.0),
        ("remark", 20.0),
        ("EEG", 16.0),
        ("物流单号", 20.0),
        ("erpOrderNum", 20.0),
        ("ocrModify", 12.0),
        ("modifyStatus", 14.0),
        ("introduceInvoiceFlag", 18.0),
        ("是否交旧", 12.0),
        ("是否自提", 12.0),
        ("receiverName", 16.0),
        ("productCode", 18.0),
        ("subsideAmt", 14.0),
        ("productName", 28.0),
        ("交旧品类", 16.0),
        ("收货地址是否农村地区", 20.0),
        ("airConditionerKitInfo", 22.0),
        ("开票日期", 14.0),
    ];

    for (col, (_, w)) in col_defs.iter().enumerate() {
        ws.set_column_width(col as u16, *w)
            .map_err(|e| e.to_string())?;
    }

    // Row 1: Header
    ws.set_row_height(0, 30.0).map_err(|e| e.to_string())?;
    for (col, (name, _)) in col_defs.iter().enumerate() {
        let fmt = if [4, 7, 8, 10, 11, 12, 17, 24, 42, 47, 48, 49, 50, 51, 57, 59].contains(&col) {
            &s.col_header_center
        } else {
            &s.col_header
        };
        ws.write_string_with_format(0, col as u16, *name, fmt)
            .map_err(|e| e.to_string())?;
    }

    // Pre-resolve column mappings for Appliance and Digital
    let app_h = &app_up.header;
    let dig_h = &dig_up.header;

    // Build output-column index extractor parameterized by category
    let resolve_indices =
        |h: &HeaderMap, is_dig: bool, label: &str| -> Result<Vec<ReportColumn>, String> {
            (0..col_defs.len())
                .map(|col| {
                    let optional =
                        (!is_dig && matches!(col, 25 | 26)) || (is_dig && matches!(col, 44 | 58));
                    let index = match col {
                        22 => h.find_occurrence("图片1", 1),
                        25 | 26 => {
                            if is_dig {
                                h.find(&[col_defs[col].0])
                            } else {
                                None
                            }
                        }
                        27 => h.find_occurrence("图片1", 2),
                        31 => h.find(&["img5", "图片5"]),
                        32 => h.find(&["img6", "图片6"]),
                        44 => {
                            if is_dig {
                                None
                            } else {
                                h.find(&["EEG"])
                            }
                        }
                        56 => h.find(&["交旧品类", "oldExchangeType"]),
                        58 => {
                            if is_dig {
                                None
                            } else {
                                h.find(&["airConditionerKitInfo"])
                            }
                        }
                        _ => h.find(&[col_defs[col].0]),
                    };
                    if index.is_none() && !optional {
                        return Err(format!("{}: 缺少必要列 [{}]", label, col_defs[col].0));
                    }
                    let mut column = ReportColumn::new(index, col_defs[col].0);
                    if col_defs[col].0 == "状态" {
                        column.kind = CellKind::CenteredText;
                    }
                    Ok(column)
                })
                .collect()
        };

    let app_map = resolve_indices(app_h, false, "已上传家电电脑.xlsx")?;
    let dig_map = resolve_indices(dig_h, true, "已上传数码.xlsx")?;

    let mut cur_row: u32 = 1;

    let write_dataset = |ws: &mut Worksheet,
                         rows: &[Vec<Data>],
                         col_map: &[ReportColumn],
                         cur_row: &mut u32|
     -> Result<(), String> {
        for row in &rows[1..] {
            ws.set_row_height(*cur_row, 22.0)
                .map_err(|e| e.to_string())?;
            for (col, column) in col_map.iter().enumerate() {
                column
                    .kind
                    .write(ws, s, *cur_row, col as u16, column.cell(row))?;
            }
            *cur_row += 1;
        }
        Ok(())
    };

    write_dataset(ws, app_up, &app_map, &mut cur_row)?;
    write_dataset(ws, dig_up, &dig_map, &mut cur_row)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use calamine::{Reader, open_workbook_auto};

    fn finance_fixture(label: &str) -> std::path::PathBuf {
        use crate::jobs::{refund::OUTPUT_FIELDS, unionpay::HEADERS, uploaded};
        let root = crate::test_support::unique_temp_path(label);
        std::fs::create_dir_all(&root).unwrap();
        for (name, tail, merchant) in [
            (
                "已上传家电电脑.xlsx",
                &uploaded::APPLIANCE_TAIL,
                "89813015722APT1",
            ),
            (
                "已上传数码.xlsx",
                &uploaded::DIGITAL_TAIL,
                "89813014812B06R",
            ),
        ] {
            let headers: Vec<_> = uploaded::FRONT_HEADERS
                .iter()
                .copied()
                .chain(tail.iter().map(|field| field.synonyms[0]))
                .chain(["补贴金额"])
                .collect();
            write_fixture(
                &root,
                name,
                &headers,
                &[&[("商户号", merchant), ("状态", "待审核")]],
            );
        }
        for (name, merchant) in [
            ("回款明细家电电脑.xlsx", "89813015722APT1"),
            ("回款明细数码.xlsx", "89813014812B06R"),
        ] {
            write_fixture(&root, name, &OUTPUT_FIELDS, &[&[("核销商编", merchant)]]);
        }
        write_fixture(
            &root,
            "销售用券情况统计.xlsx",
            &["数电发票号码", "财务大类", "品牌", "商品名称"],
            &[],
        );
        write_fixture(
            &root,
            "发票明细.xlsx",
            &["数电发票号码", "开票类型", "开票状态"],
            &[],
        );
        write_fixture(&root, "银联交易明细门店.xlsx", &HEADERS, &[]);
        root
    }

    fn write_fixture(root: &Path, name: &str, headers: &[&str], rows: &[&[(&str, &str)]]) {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        for (col, header) in headers.iter().enumerate() {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (row, values) in rows.iter().enumerate() {
            for (field, value) in *values {
                let col = headers.iter().position(|header| header == field).unwrap();
                sheet
                    .write_string(row as u32 + 1, col as u16, *value)
                    .unwrap();
            }
        }
        book.save(root.join(name)).unwrap();
    }

    #[test]
    fn finance_report_aligns_reordered_refunds_by_header() {
        use crate::jobs::refund::OUTPUT_FIELDS;
        let root = finance_fixture("finance-reordered");
        let mut app_headers = OUTPUT_FIELDS;
        app_headers.rotate_left(3);
        let mut dig_headers = OUTPUT_FIELDS;
        dig_headers.swap(10, 17);
        for (name, headers, merchant, product) in [
            (
                "回款明细家电电脑.xlsx",
                &app_headers,
                "89813015722APT1",
                "冰箱A",
            ),
            (
                "回款明细数码.xlsx",
                &dig_headers,
                "89813014812B06R",
                "手机B",
            ),
        ] {
            write_fixture(
                &root,
                name,
                headers,
                &[&[
                    ("核销商编", merchant),
                    ("补贴金额", "123.45"),
                    ("商品名称", product),
                    ("交易完成时间", "2025-10-13 12:00:00"),
                    ("补贴比例", "0.15"),
                ]],
            );
        }
        let path = root.join("finance.xlsx");
        generate_store_finance_workbook(&root, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        let range = book
            .worksheet_range("2.门店累计回款表（事业部财务下发，门店筛选自己的）")
            .unwrap();
        let rows: Vec<_> = range.rows().collect();
        for (col, field) in app_headers.iter().enumerate() {
            assert_eq!(cell_to_string(&rows[0][col]), *field);
        }
        let h = HeaderMap::from_header_row(rows[0]);
        for (row, product) in [(rows[1], "冰箱A"), (rows[2], "手机B")] {
            assert_eq!(
                row[h.require("补贴金额", "output").unwrap()],
                Data::Float(123.45)
            );
            assert_eq!(
                cell_to_string(&row[h.require("商品名称", "output").unwrap()]),
                product
            );
            assert_eq!(
                row[h.require("补贴比例", "output").unwrap()],
                Data::Float(0.15)
            );
            assert!(matches!(
                row[h.require("交易完成时间", "output").unwrap()],
                Data::DateTime(_)
            ));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finance_report_formats_reordered_store_fields_by_name() {
        let root = finance_fixture("finance-reordered-store");
        let mut headers = crate::jobs::unionpay::HEADERS;
        headers.reverse();
        write_fixture(
            &root,
            "银联交易明细门店.xlsx",
            &headers,
            &[&[
                ("检索号", "00123456789N"),
                ("交易时间", "2026-09-14 10:18:09"),
                ("交易金额", "-123.45"),
                ("商户订单号", "000123"),
            ]],
        );
        let path = root.join("finance.xlsx");
        generate_store_finance_workbook(&root, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        for sheet in [0, 1] {
            let range = book.worksheet_range_at(sheet).unwrap().unwrap();
            let rows: Vec<_> = range.rows().collect();
            let header = HeaderMap::from_header_row(rows[0]);
            assert_eq!(
                rows[1][header.require("交易金额", "output").unwrap()],
                Data::Float(-123.45)
            );
            assert_eq!(
                rows[1][header.require("商户订单号", "output").unwrap()],
                "000123"
            );
            assert!(matches!(
                rows[1][header.require("交易时间", "output").unwrap()],
                Data::DateTime(_)
            ));
            assert_eq!(
                rows[1][header.require("清算金额", "output").unwrap()],
                Data::Empty
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finance_report_preserves_empty_zero_and_signed_amounts() {
        let root = finance_fixture("finance-empty-money");
        write_fixture(
            &root,
            "银联交易明细门店.xlsx",
            &crate::jobs::unionpay::HEADERS,
            &[
                &[("检索号", "empty")],
                &[("检索号", "zero"), ("清算金额", "0")],
                &[("检索号", "negative"), ("清算金额", "-12.34")],
                &[("检索号", "positive"), ("清算金额", "56.78")],
            ],
        );
        let path = root.join("finance.xlsx");
        generate_store_finance_workbook(&root, &path).unwrap();
        let mut book = open_workbook_auto(&path).unwrap();
        let final_rows = book
            .worksheet_range("最终匹配表（全部的国补发生数据上匹配）")
            .unwrap();
        let store_rows = book
            .worksheet_range("1.门店国补发生表（银联系统直接导出，不需要加工）")
            .unwrap();
        for (row, expected) in [
            (1, Data::Empty),
            (2, Data::Float(0.0)),
            (3, Data::Float(-12.34)),
            (4, Data::Float(56.78)),
        ] {
            assert_eq!(final_rows.get_value((row, 7)), Some(&expected));
            assert_eq!(store_rows.get_value((row, 6)), Some(&expected));
        }
        for col in [6, 8, 20] {
            assert_eq!(final_rows.get_value((1, col)), Some(&Data::Empty));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn final_products_use_invoice_first_row_and_full_name() {
        let strings = |values: &[&str]| {
            values
                .iter()
                .map(|value| Data::String((*value).to_string()))
                .collect::<Vec<_>>()
        };
        let headers = strings(&[
            "清算时间",
            "交易时间",
            "终端号",
            "交易类型",
            "卡号",
            "交易金额",
            "清算金额",
            "手续费",
            "流水号",
            "检索号",
            "卡类型",
            "发卡行",
            "商户号",
            "商户名称",
            "分店简称",
            "商户订单号",
            "银商订单号",
            "交易方式",
            "分店",
            "优惠金额",
            "备注",
        ]);
        let mut store = vec![headers.clone()];
        for (reference, remark) in [
            ("ref-a", "已退货"),
            ("ref-b", ""),
            ("ref-c", ""),
            ("ref-d", ""),
        ] {
            let mut row = vec![Data::Empty; headers.len()];
            row[9] = Data::String(reference.to_string());
            row[16] = Data::String("order-that-must-not-match".to_string());
            row[20] = Data::String(remark.to_string());
            store.push(row);
        }
        // Reordered sales columns and a reference decoy ensure only invoices join.
        let sales = vec![
            strings(&["商品名称", "品牌", "数电发票号码", "财务大类", "参考号"]),
            strings(&[
                "海尔 冰箱 BCD-123 完整商品名称",
                "海尔",
                "invoice-a",
                "冰箱",
                "different-ref",
            ]),
            strings(&[
                "later product",
                "later brand",
                "invoice-a",
                "later category",
                "ref-a",
            ]),
            strings(&["blank invoice product", "decoy", "", "decoy", "ref-b"]),
            strings(&[
                "reference decoy",
                "decoy",
                "another-invoice",
                "decoy",
                "ref-c",
            ]),
        ];
        let uploads = vec![
            strings(&["检索参考号", "状态", "发票号码", "补贴金额"]),
            strings(&["ref-a", "审核通过未回款", "invoice-a", "1"]),
            strings(&["ref-b", "待审核", "", "1"]),
            strings(&["ref-c", "审核失败", "unmatched-invoice", "1"]),
        ];
        let digital = vec![uploads[0].clone()];
        let invoices = vec![
            strings(&["数电发票号码", "开票类型", "开票状态"]),
            strings(&["invoice-a", "红票", "开票完成"]),
        ];
        let mut workbook = Workbook::new();
        build_final_match_sheet(
            &mut workbook,
            &StylePool::default(),
            &SheetData::new(store.clone()),
            &SheetData::new(sales.clone()),
            &SheetData::new(uploads.clone()),
            &SheetData::new(digital.clone()),
            &SheetData::new(invoices.clone()),
        )
        .unwrap();
        let path =
            std::env::temp_dir().join(format!("subsidy-products-{}.xlsx", std::process::id()));
        workbook.save(&path).unwrap();
        let mut output = open_workbook_auto(&path).unwrap();
        let range = output
            .worksheet_range("最终匹配表（全部的国补发生数据上匹配）")
            .unwrap();
        assert_eq!(range.width(), 27);
        let rows = range.rows().collect::<Vec<_>>();
        assert_eq!(rows[0][23], "产品型号");
        assert_eq!(rows[1][21], "冰箱");
        assert_eq!(rows[1][22], "海尔");
        assert_eq!(rows[1][23], "海尔 冰箱 BCD-123 完整商品名称");
        assert_eq!(rows[1][24], "已退货");
        assert_eq!(rows[1][25], "invoice-a");
        assert_eq!(rows[1][26], "是");
        for row in &rows[2..] {
            assert!(
                row[21..24]
                    .iter()
                    .all(|cell| cell_to_string(cell).is_empty())
            );
        }
        assert_eq!(rows[2][24], "待审核");
        assert_eq!(rows[3][25], "unmatched-invoice");
        assert_eq!(rows[4][24], "未提交");
        std::fs::remove_file(path).unwrap();

        let mut invalid_sales = sales.clone();
        invalid_sales[0][2] = Data::String("wrong invoice column".to_string());
        let error = build_final_match_sheet(
            &mut Workbook::new(),
            &StylePool::default(),
            &SheetData::new(store.clone()),
            &SheetData::new(invalid_sales.clone()),
            &SheetData::new(uploads.clone()),
            &SheetData::new(digital.clone()),
            &SheetData::new(invoices.clone()),
        )
        .unwrap_err();
        assert!(error.contains("销售用券情况统计.xlsx: 缺少必要列 [数电发票号码]"));
    }
}
