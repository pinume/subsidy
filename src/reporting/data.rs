use super::reader::{HeaderMap, cell_to_string, extract_unique_merchant_code, read_sheet_rows};
use calamine::Data;
use std::ops::Deref;
use std::path::Path;

pub(crate) struct SheetData {
    rows: Vec<Vec<Data>>,
    pub header: HeaderMap,
}

impl SheetData {
    pub fn new(rows: Vec<Vec<Data>>) -> Self {
        let header = HeaderMap::from_header_row(&rows[0]);
        Self { rows, header }
    }
    pub fn load(path: &Path, uploaded: bool) -> Result<Self, String> {
        let mut sheet = Self::new(read_sheet_rows(path)?);
        if uploaded {
            let status = sheet.header.require("状态", &path.display().to_string())?;
            for row in sheet.rows.iter_mut().skip(1) {
                if let Some(cell) = row.get_mut(status)
                    && cell_to_string(cell) == "审核终止"
                {
                    *cell = Data::String("审核失败".into());
                }
            }
        }
        Ok(sheet)
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
        let sales = SheetData::load(&input.join("销售用券情况统计.xlsx"), false)?;
        let app_upload = SheetData::load(&input.join("已上传家电电脑.xlsx"), true)?;
        let dig_upload = SheetData::load(&input.join("已上传数码.xlsx"), true)?;
        let invoices = SheetData::load(&input.join("发票明细.xlsx"), false)?;
        let app_refund = SheetData::load(&input.join("回款明细家电电脑.xlsx"), false)?;
        let dig_refund = SheetData::load(&input.join("回款明细数码.xlsx"), false)?;
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
    fn shared_inputs_are_loaded_once_and_normalize_uploads_without_changing_sources() {
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
        assert_eq!(inputs.app_upload[1][1], "审核失败");
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
