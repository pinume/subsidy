use calamine::{Data, DataType};
use rust_xlsxwriter::{ExcelDateTime, Format, Worksheet};

use super::reader::{cell_to_decimal, cell_to_string};
use super::styles::StylePool;

pub fn write_date_cell(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    cell: &Data,
    format: &Format,
) -> Result<(), String> {
    if matches!(cell, Data::Empty) {
        return ws
            .write_string_with_format(row, col, "", format)
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    if let Some(value) = cell.as_datetime() {
        return ws
            .write_datetime_with_format(row, col, value, format)
            .map(|_| ())
            .map_err(|e| e.to_string());
    }

    let value = cell_to_string(cell);
    match ExcelDateTime::parse_from_str(&value) {
        Ok(datetime) => ws
            .write_datetime_with_format(row, col, datetime, format)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        Err(error) => {
            eprintln!(
                "[警告] 第 {} 行第 {} 列日期值 '{}' 无法解析，已保留原文本: {}",
                row + 1,
                col + 1,
                value,
                error
            );
            ws.write_string_with_format(row, col, value, format)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

pub fn write_refund_cell(
    ws: &mut Worksheet,
    s: &StylePool,
    row: u32,
    col: u16,
    cell: &Data,
) -> Result<(), String> {
    match col {
        1 => {
            write_date_cell(ws, row, col, cell, &s.datetime)?;
        }
        8 | 9 | 10 | 18 => {
            if let Some(dec) = cell_to_decimal(cell) {
                ws.write_with_format(row, col, dec, &s.money)
                    .map_err(|e| e.to_string())?;
            } else {
                ws.write_string_with_format(row, col, "", &s.text_right)
                    .map_err(|e| e.to_string())?;
            }
        }
        11 => {
            if let Some(dec) = cell_to_decimal(cell) {
                ws.write_with_format(row, col, dec, &s.percent)
                    .map_err(|e| e.to_string())?;
            } else {
                ws.write_string_with_format(row, col, "", &s.text_right)
                    .map_err(|e| e.to_string())?;
            }
        }
        _ => {
            let val = cell_to_string(cell);
            ws.write_string_with_format(row, col, val, &s.text_left)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
