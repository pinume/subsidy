use calamine::{Data, DataType};
use rust_xlsxwriter::{ExcelDateTime, Format, Worksheet};

use super::data::CellKind;
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

pub fn write_decimal_cell(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    cell: &Data,
    format: &Format,
    empty_format: &Format,
) -> Result<(), String> {
    match cell_to_decimal(cell) {
        Some(value) => ws.write_with_format(row, col, value, format),
        None => ws.write_string_with_format(row, col, "", empty_format),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

impl CellKind {
    pub fn write(
        self,
        ws: &mut Worksheet,
        s: &StylePool,
        row: u32,
        col: u16,
        cell: &Data,
    ) -> Result<(), String> {
        match self {
            Self::Date => write_date_cell(ws, row, col, cell, &s.date),
            Self::DateTime => write_date_cell(ws, row, col, cell, &s.datetime),
            Self::Money => write_decimal_cell(ws, row, col, cell, &s.money, &s.text_right),
            Self::Percent => write_decimal_cell(ws, row, col, cell, &s.percent, &s.text_right),
            Self::Text | Self::CenteredText => {
                let format = if matches!(self, Self::CenteredText) {
                    &s.text_center
                } else {
                    &s.text_left
                };
                ws.write_string_with_format(row, col, cell_to_string(cell), format)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }
    }
}
