use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use calamine::{DataType, Reader, Xlsx, open_workbook};
use rust_xlsxwriter::{Format, Workbook};
use subsidy::jobs::{Job, unionpay::UnionPayJob};
use subsidy::model::Value;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const HEADERS: [&str; 26] = [
    "清算时间",
    "交易时间",
    "终端号",
    "交易类型",
    "卡号",
    "交易金额",
    "清算金额",
    "手续费",
    "T0手续费",
    "D1手续费",
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
    "分期手续费",
    "付款附言",
    "备注",
    "买家ID",
];

fn input_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "subsidy-reading-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let input = root.join("raw");
    std::fs::create_dir_all(&input).unwrap();
    input
}

fn source(input: &Path, id: u32, serial: f64) -> PathBuf {
    let path = input.join(format!("89813014812B06R_MX_20261007000000_{id}.xlsx"));
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.write_string(0, 0, "汇总").unwrap();
    for (col, header) in HEADERS.iter().enumerate() {
        sheet.write_string(1, col as u16, *header).unwrap();
    }
    let format = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
    sheet
        .write_number_with_format(2, 0, serial, &format)
        .unwrap();
    sheet
        .write_number_with_format(2, 1, serial + 0.5, &format)
        .unwrap();
    sheet.write_string(2, 3, "消费").unwrap();
    sheet.write_number(2, 5, 100.0).unwrap();
    sheet.write_string(2, 10, "SN001").unwrap();
    sheet.write_string(2, 11, "REF001").unwrap();
    sheet.write_string(2, 14, "89813014812B06R").unwrap();
    sheet
        .write_string(
            3,
            0,
            "请注意：D1手续费字段为预估数据仅供参考，实际以17：40分之后的D1划付数据为准。",
        )
        .unwrap();
    book.save(&path).unwrap();
    path
}

fn use_1904_epoch(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let mut archive = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if entry.name() == "xl/workbook.xml" {
            let xml = String::from_utf8(data).unwrap();
            assert!(xml.contains("<workbookPr "));
            data = xml
                .replacen("<workbookPr ", "<workbookPr date1904=\"1\" ", 1)
                .into_bytes();
        }
        writer
            .start_file(entry.name(), SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&data).unwrap();
    }
    std::fs::write(path, writer.finish().unwrap().into_inner()).unwrap();
}

fn run_cli(input: &Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", input.display()).unwrap();
    let output = child.wait_with_output().unwrap();
    // Other seven categories are intentionally absent.
    assert_eq!(output.status.code(), Some(1));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn cli_preserves_1904_dates_through_cleaning_and_publication() {
    let input = input_dir("1904");
    let path = source(&input, 1, 44481.0);
    use_1904_epoch(&path);
    let before = std::fs::read(&path).unwrap();
    let table = UnionPayJob.run(&input).unwrap();
    let date = chrono::NaiveDate::from_ymd_opt(2025, 10, 13).unwrap();
    assert_eq!(table.rows[0].values[0], Value::Date(date));
    assert_eq!(
        table.rows[0].values[1],
        Value::DateTime(date.and_hms_opt(12, 0, 0).unwrap())
    );
    assert!(run_cli(&input).contains("[门店银联交易明细] 处理成功"));
    let mut book: Xlsx<_> = open_workbook(
        input
            .parent()
            .unwrap()
            .join("国补明细/银联交易明细门店.xlsx"),
    )
    .unwrap();
    let range = book.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.get_value((1, 0)).unwrap().as_date(), Some(date));
    assert_eq!(
        range.get_value((1, 1)).unwrap().as_datetime(),
        date.and_hms_opt(12, 0, 0)
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
    std::fs::remove_dir_all(input.parent().unwrap()).unwrap();
}

#[test]
fn cli_accepts_exports_differing_only_in_native_dates() {
    let input = input_dir("dates");
    source(&input, 1, 45943.0);
    source(&input, 2, 45944.0);
    assert!(run_cli(&input).contains("[门店银联交易明细] 处理成功"));
    let mut book: Xlsx<_> = open_workbook(
        input
            .parent()
            .unwrap()
            .join("国补明细/银联交易明细门店.xlsx"),
    )
    .unwrap();
    let range = book.worksheet_range("Sheet1").unwrap();
    assert_eq!(range.height(), 3);
    assert_ne!(range.get_value((1, 0)), range.get_value((2, 0)));
    std::fs::remove_dir_all(input.parent().unwrap()).unwrap();
}
