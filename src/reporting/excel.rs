use calamine::{Data, DataType};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, Worksheet, XlsxError};

use super::data::{CellKind, ReportColumn};
use super::reader::{cell_to_decimal, cell_to_string};
use super::styles::StylePool;

/// 报表写出错误：统一承接 xlsx 写出错误与既有文本错误，可用 `?` 转回调用方的 `String`。
#[derive(Debug)]
pub struct ReportError(String);

impl From<XlsxError> for ReportError {
    fn from(error: XlsxError) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for ReportError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for ReportError {
    fn from(message: &str) -> Self {
        Self(message.to_string())
    }
}

impl From<ReportError> for String {
    fn from(error: ReportError) -> Self {
        error.0
    }
}

pub fn write_date_cell(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    cell: &Data,
    format: &Format,
) -> Result<(), ReportError> {
    if matches!(cell, Data::Empty) {
        ws.write_string_with_format(row, col, "", format)?;
        return Ok(());
    }

    if let Some(value) = cell.as_datetime() {
        ws.write_datetime_with_format(row, col, value, format)?;
        return Ok(());
    }

    let value = cell_to_string(cell);
    match ExcelDateTime::parse_from_str(&value) {
        Ok(datetime) => {
            ws.write_datetime_with_format(row, col, datetime, format)?;
        }
        Err(error) => {
            eprintln!(
                "[警告] 工作表「{}」第 {} 行第 {} 列日期值 '{}' 无法解析，已保留原文本: {}",
                ws.name(),
                row + 1,
                col + 1,
                value,
                error
            );
            ws.write_string_with_format(row, col, value, format)?;
        }
    }
    Ok(())
}

/// 与 `write_date_cell` 相同的日期有效性判断：原生日期或当前解析器可识别的文本。
pub(crate) fn is_date_cell(cell: &Data) -> bool {
    cell.as_datetime().is_some() || ExcelDateTime::parse_from_str(&cell_to_string(cell)).is_ok()
}

pub fn write_decimal_cell(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    cell: &Data,
    format: &Format,
    empty_format: &Format,
) -> Result<(), ReportError> {
    match cell_to_decimal(cell) {
        Some(value) => ws.write_with_format(row, col, value, format),
        // 无法解析的金额保留原文本（报表规则 §3.3），空值留空。
        None => ws.write_string_with_format(row, col, cell_to_string(cell), empty_format),
    }?;
    Ok(())
}

impl CellKind {
    pub fn write(
        self,
        ws: &mut Worksheet,
        s: &StylePool,
        row: u32,
        col: u16,
        cell: &Data,
    ) -> Result<(), ReportError> {
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
                ws.write_string_with_format(row, col, cell_to_string(cell), format)?;
                Ok(())
            }
        }
    }
}

fn write_report_row(
    ws: &mut Worksheet,
    styles: &StylePool,
    index: u32,
    row: &[Data],
    columns: &[ReportColumn],
) -> Result<(), ReportError> {
    ws.set_row_height(index, 22.0)?;
    for (col, column) in columns.iter().enumerate() {
        column
            .kind
            .write(ws, styles, index, col as u16, column.cell(row))?;
    }
    Ok(())
}

/// 报表工作表构建器：统一封装工作表初始化、列宽配置、标题头、章节头及行流式写入
pub struct ReportSheetBuilder<'a> {
    pub ws: &'a mut Worksheet,
    pub styles: &'a StylePool,
    pub cur_row: u32,
}

impl<'a> ReportSheetBuilder<'a> {
    pub fn new(
        wb: &'a mut Workbook,
        name: &str,
        styles: &'a StylePool,
    ) -> Result<Self, ReportError> {
        let ws = wb.add_worksheet();
        ws.set_name(name)?;
        Ok(Self {
            ws,
            styles,
            cur_row: 0,
        })
    }

    pub fn set_column_widths(&mut self, widths: &[f64]) -> Result<&mut Self, ReportError> {
        for (col, &w) in widths.iter().enumerate() {
            self.ws.set_column_width(col as u16, w)?;
        }
        Ok(self)
    }

    pub fn set_freeze_panes(&mut self, row: u32, col: u16) -> Result<&mut Self, ReportError> {
        self.ws.set_freeze_panes(row, col)?;
        Ok(self)
    }

    pub fn write_title_block(
        &mut self,
        max_col: u16,
        title: &str,
        subtitle: &str,
    ) -> Result<&mut Self, ReportError> {
        self.ws.set_row_height(0, 30.0)?;
        self.ws
            .merge_range(0, 0, 0, max_col, title, &self.styles.title)?;

        self.ws.set_row_height(1, 22.0)?;
        self.ws
            .merge_range(1, 0, 1, max_col, subtitle, &self.styles.subtitle)?;

        self.ws.set_row_height(2, 8.0)?;
        self.cur_row = 3;
        Ok(self)
    }

    pub fn write_section_header(
        &mut self,
        max_col: u16,
        title: &str,
    ) -> Result<&mut Self, ReportError> {
        self.ws.set_row_height(self.cur_row, 24.0)?;
        self.ws.merge_range(
            self.cur_row,
            0,
            self.cur_row,
            max_col,
            title,
            &self.styles.section_header,
        )?;
        self.cur_row += 1;
        Ok(self)
    }

    pub fn write_headers(&mut self, headers: &[&str]) -> Result<&mut Self, ReportError> {
        self.write_headers_with(headers, |_| false)
    }

    pub fn write_headers_with<F>(
        &mut self,
        headers: &[&str],
        mut is_center: F,
    ) -> Result<&mut Self, ReportError>
    where
        F: FnMut(usize) -> bool,
    {
        self.ws.set_row_height(self.cur_row, 30.0)?;
        for (col, h) in headers.iter().enumerate() {
            let fmt = if is_center(col) {
                &self.styles.col_header_center
            } else {
                &self.styles.col_header
            };
            self.ws
                .write_string_with_format(self.cur_row, col as u16, *h, fmt)?;
        }
        self.cur_row += 1;
        Ok(self)
    }

    pub fn write_blank_row(&mut self, height: f64) -> Result<&mut Self, ReportError> {
        self.ws.set_row_height(self.cur_row, height)?;
        self.cur_row += 1;
        Ok(self)
    }

    pub fn write_row(
        &mut self,
        row: &[Data],
        columns: &[ReportColumn],
    ) -> Result<&mut Self, ReportError> {
        write_report_row(self.ws, self.styles, self.cur_row, row, columns)?;
        self.cur_row += 1;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unparsable_money_text_is_kept_instead_of_scaled_or_blanked() {
        let path = crate::test_support::unique_temp_path("money-text").with_extension("xlsx");
        let styles = StylePool::default();
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        for (row, cell) in [
            Data::String("3000,00".into()),
            Data::String("12.5".into()),
            Data::Empty,
        ]
        .iter()
        .enumerate()
        {
            CellKind::Money
                .write(ws, &styles, row as u32, 0, cell)
                .unwrap();
        }
        wb.save(&path).unwrap();
        let rows = super::super::reader::read_sheet_rows(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        // “3000,00”以逗号作小数点，不得删除逗号变成 300000。
        assert_eq!(rows[0][0], Data::String("3000,00".into()));
        assert_eq!(rows[1][0], Data::Float(12.5));
        // 空值留空，不写 0：读回时该行不在数据区内。
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn report_sheet_builder_initializes_and_tracks_row_flow() {
        let mut wb = Workbook::new();
        let styles = StylePool::default();
        let mut builder = ReportSheetBuilder::new(&mut wb, "测试表", &styles).unwrap();
        assert_eq!(builder.cur_row, 0);

        builder.set_column_widths(&[10.0, 20.0]).unwrap();
        builder
            .write_title_block(1, "主标题", "副标题说明文字")
            .unwrap();
        assert_eq!(builder.cur_row, 3);

        builder.write_section_header(1, "第一节").unwrap();
        assert_eq!(builder.cur_row, 4);

        builder.write_headers(&["列1", "列2"]).unwrap();
        assert_eq!(builder.cur_row, 5);

        let row_data = [Data::String("值1".into()), Data::String("值2".into())];
        let cols = [
            ReportColumn::new(Some(0), "列1"),
            ReportColumn::new(Some(1), "列2"),
        ];
        builder.write_row(&row_data, &cols).unwrap();
        assert_eq!(builder.cur_row, 6);

        builder.write_blank_row(8.0).unwrap();
        assert_eq!(builder.cur_row, 7);
    }
}
