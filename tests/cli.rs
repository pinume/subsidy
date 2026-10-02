use std::io::{Read, Write};
use std::process::{Command, Stdio};

use rust_xlsxwriter::Workbook;
use subsidy::io::xlsx_reader::{RawCell, open_sheets};

fn cli_result(input: &str) -> (bool, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap() + &String::from_utf8_lossy(&output.stderr),
    )
}

fn run_cli(input: &str) -> String {
    cli_result(input).1
}

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

/// 通过真实终端入口验证自动批量处理、失败后继续与输出文件内容。
#[test]
fn cli_runs_all_categories_without_menu_and_publishes_to_detail_dir() {
    let base = std::env::temp_dir().join(format!(
        "clean-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let input_dir = base.join("输入目录");
    std::fs::create_dir_all(&input_dir).unwrap();

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("对账数据").unwrap();
    sheet.write_string(0, 0, "汇总").unwrap();
    for (col, header) in HEADERS.iter().enumerate() {
        sheet.write_string(1, col as u16, *header).unwrap();
    }
    let row: [&str; 26] = [
        "20260914",
        "2026-09-14 10:18:09",
        "T001",
        "消费",
        "622***1234",
        "100.00",
        "100.00",
        "1.00",
        "0.50",
        "0.50",
        "SN0001",
        "16867252734N",
        "借记卡",
        "工商银行",
        "89813014812B06R",
        "某门店",
        "门店简",
        "ORDER1",
        "AC0001",
        "云闪付",
        "分店A",
        "0.00",
        "0.00",
        "备注文字",
        "",
        "buyer001",
    ];
    for (col, value) in row.iter().enumerate() {
        sheet.write_string(2, col as u16, *value).unwrap();
    }
    sheet
        .write_string(
            3,
            0,
            "请注意：D1手续费字段为预估数据仅供参考，实际以17：40分之后的D1划付数据为准。",
        )
        .unwrap();
    workbook
        .save(input_dir.join("89813014812B06R_MX_20260914101809_1.xlsx"))
        .unwrap();

    let source = input_dir.join("89813014812B06R_MX_20260914101809_1.xlsx");
    let source_before = std::fs::read(&source).unwrap();
    let old_output = base.join("source_data");
    std::fs::create_dir_all(&old_output).unwrap();
    let old_result = old_output.join("银联交易明细所有.xlsx");
    std::fs::write(&old_result, b"historical result").unwrap();
    for (folder, name) in [
        ("cleaned", "银联交易明细门店.xlsx"),
        ("subsidy_summary", "国补上传情况汇总.xlsx"),
    ] {
        std::fs::create_dir(base.join(folder)).unwrap();
        std::fs::write(base.join(folder).join(name), b"legacy result").unwrap();
    }
    // A corrupt retired-category file must be ignored, rather than produce a failure.
    std::fs::write(input_dir.join("银联国补明细1.xlsx"), b"retired data").unwrap();
    let cleaned = base.join("国补明细");
    std::fs::create_dir_all(&cleaned).unwrap();
    let stale = cleaned.join("发票明细.xlsx");
    std::fs::write(&stale, b"stale invoice").unwrap();
    let reports = base.join("国补报表");
    std::fs::create_dir_all(&reports).unwrap();
    for name in [
        "国补上传情况汇总.xlsx",
        "国补上传情况汇总.md",
        "26年国补门店财务统筹表.xlsx",
    ] {
        std::fs::write(reports.join(name), b"old report").unwrap();
    }
    let input = format!(
        "\n{}\n  \"{}\"  \n",
        base.join("missing").display(),
        input_dir.display()
    );
    let (ok, output) = cli_result(&input);
    assert!(!ok);
    assert!(output.contains("路径为空"));
    assert!(output.contains("路径不存在"));
    assert!(!output.contains("菜单"));
    assert!(!output.contains("请输入编号"));
    assert!(!output.contains("银联国补"));
    let outcomes: Vec<_> = output
        .lines()
        .filter(|line| line.contains("处理成功") || line.contains("处理失败"))
        .filter_map(|line| line.find('[').map(|start| &line[start..]))
        .collect();
    let titles = [
        "发票明细",
        "数码已上传数据",
        "家电、电脑已上传数据",
        "门店银联交易明细",
        "家电电脑回款明细",
        "数码回款明细",
        "收款单统计",
        "销售用券情况统计",
    ];
    assert_eq!(outcomes.len(), titles.len());
    for (line, title) in outcomes.iter().zip(titles) {
        assert!(line.starts_with(&format!("[{title}]")), "{line}");
    }
    assert_eq!(output.matches("处理失败").count(), 7);
    assert_eq!(output.matches("处理成功").count(), 1);
    assert!(output.contains("全部类别执行完毕：共 8 项，成功 1，失败 7。"));

    let output_path = base.join("国补明细").join("银联交易明细门店.xlsx");
    assert!(
        output_path.is_file(),
        "应在输入目录同级的 国补明细/ 下生成结果文件"
    );

    // 确认写出的文件是一个真正可被读回的 XLSX（而不仅仅是存在同名文件）。
    let sheets = open_sheets(&output_path).unwrap();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].row_texts(1), HEADERS.to_vec());
    assert_eq!(sheets[0].last_value_row(), Some(2));
    assert_eq!(sheets[0].cell(2, 6), RawCell::Float(100.0));
    assert_eq!(sheets[0].cell(2, 12), RawCell::Text("16867252734N".into()));
    assert_eq!(sheets[0].cell(2, 18), RawCell::Text("ORDER1".into()));

    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert_eq!(std::fs::read(&old_result).unwrap(), b"historical result");
    assert!(!base.join("国补明细/银联交易明细所有.xlsx").exists());
    assert_eq!(std::fs::read_dir(base.join("国补明细")).unwrap().count(), 3);
    assert_eq!(std::fs::read(&stale).unwrap(), b"stale invoice");
    assert_eq!(output.matches("跳过报表").count(), 2);
    assert!(output.contains("不使用旧文件"));
    for name in [
        "国补上传情况汇总.xlsx",
        "国补上传情况汇总.md",
        "26年国补门店财务统筹表.xlsx",
    ] {
        assert_eq!(std::fs::read(reports.join(name)).unwrap(), b"old report");
    }
    assert_eq!(
        run_cli(&format!("{}\n", input_dir.display()))
            .matches("处理成功")
            .count(),
        1
    );
    assert_eq!(
        open_sheets(&output_path).unwrap()[0].row_texts(1),
        HEADERS.to_vec()
    );
    // A readable first sheet must not hide a later sheet's parse failure.
    workbook
        .add_worksheet()
        .set_name("损坏工作表")
        .unwrap()
        .write_string(0, 0, "说明")
        .unwrap();
    let valid_book = workbook.save_to_buffer().unwrap();
    std::fs::write(&source, &valid_book).unwrap();
    assert_eq!(
        open_sheets(&source)
            .unwrap()
            .iter()
            .map(|sheet| sheet.name())
            .collect::<Vec<_>>(),
        ["对账数据", "损坏工作表"]
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(valid_book)).unwrap();
    let mut damaged = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if entry.name() == "xl/worksheets/sheet2.xml" {
            bytes = b"<worksheet><sheetData><row r=\"bad\"/></sheetData></worksheet>".to_vec();
        }
        damaged
            .start_file(entry.name(), zip::write::SimpleFileOptions::default())
            .unwrap();
        damaged.write_all(&bytes).unwrap();
    }
    let damaged = damaged.finish().unwrap().into_inner();
    std::fs::write(&source, &damaged).unwrap();
    let previous = std::fs::read(&output_path).unwrap();
    let (ok, log) = cli_result(&format!("{}\n", input_dir.display()));
    assert!(!ok);
    assert!(
        log.contains(&format!("{} / 损坏工作表：", source.display())),
        "{log}"
    );
    assert!(log.contains("读取失败"), "{log}");
    assert!(log.contains("[数码回款明细] 处理失败"));
    assert_eq!(std::fs::read(&source).unwrap(), damaged);
    assert_eq!(std::fs::read(&output_path).unwrap(), previous);
    std::fs::write(&source, &source_before).unwrap();
    for (folder, name) in [
        ("cleaned", "银联交易明细门店.xlsx"),
        ("subsidy_summary", "国补上传情况汇总.xlsx"),
    ] {
        assert_eq!(std::fs::read_dir(base.join(folder)).unwrap().count(), 1);
        assert_eq!(
            std::fs::read(base.join(folder).join(name)).unwrap(),
            b"legacy result"
        );
    }
    // An output directory creation error also leaves input and prior results intact.
    let saved_output = base.join("saved-cleaned");
    std::fs::rename(base.join("国补明细"), &saved_output).unwrap();
    std::fs::write(base.join("国补明细"), b"blocked output directory").unwrap();
    let failed = run_cli(&format!("{}\n", input_dir.display()));
    assert!(failed.contains("输出路径不是文件夹"));
    assert!(!failed.contains("处理成功"));
    assert!(saved_output.join("银联交易明细门店.xlsx").is_file());
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert_eq!(std::fs::read(&old_result).unwrap(), b"historical result");
    assert!(cli_result("").0);
    assert!(cli_result("\n").0);
    assert!(!run_cli("").contains("处理失败"));
    assert!(!run_cli("\n").contains("处理失败"));
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn cli_reprompts_when_output_resolves_into_source() {
    let base = std::env::temp_dir().join(format!(
        "clean-cli-overlap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let source = base.join("国补明细");
    let valid = base.join("valid");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir(&valid).unwrap();
    std::fs::write(source.join("source.xlsx"), b"original source").unwrap();
    let output = run_cli(&format!("{}\n{}\n", source.display(), valid.display()));
    assert!(output.contains("保护源数据"));
    assert_eq!(output.matches("请输入源文件夹路径").count(), 2);
    assert_eq!(output.matches("处理失败").count(), 8);
    assert_eq!(
        std::fs::read(source.join("source.xlsx")).unwrap(),
        b"original source"
    );
    assert_eq!(std::fs::read_dir(&source).unwrap().count(), 2);
    #[cfg(unix)]
    {
        let other = base.join("other");
        std::fs::create_dir(&other).unwrap();
        std::fs::rename(&source, base.join("saved-source")).unwrap();
        std::os::unix::fs::symlink(&other, &source).unwrap();
        let output = run_cli(&format!("{}\n", other.display()));
        assert!(output.contains("保护源数据"));
        assert!(!output.contains("处理失败"));
        assert_eq!(std::fs::read_dir(other).unwrap().count(), 0);
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn cli_protects_report_paths_and_rejects_an_existing_run_lock() {
    let base = std::env::temp_dir().join(format!("combine-paths-{}", std::process::id()));
    let input = base.join("raw");
    let reports = base.join("国补报表");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir(&reports).unwrap();
    let (ok, log) = cli_result(&format!("{}\n", reports.display()));
    assert!(ok); // Invalid path followed by EOF performs no work.
    assert!(log.contains("保护源数据"));
    assert!(!base.join("国补明细").exists());
    #[cfg(unix)]
    {
        std::fs::remove_dir(&reports).unwrap();
        std::os::unix::fs::symlink(&input, &reports).unwrap();
        assert!(run_cli(&format!("{}\n", input.display())).contains("保护源数据"));
        assert!(!base.join("国补明细").exists());
        std::fs::remove_file(&reports).unwrap();
        std::fs::create_dir(base.join("国补明细")).unwrap();
        std::os::unix::fs::symlink(base.join("国补明细"), &reports).unwrap();
        assert!(run_cli(&format!("{}\n", input.display())).contains("两个输出目录重叠"));
        std::fs::remove_file(&reports).unwrap();
    }
    std::fs::create_dir_all(base.join("国补明细")).unwrap();
    let lock = base.join("国补明细/.combine.lock");
    std::fs::write(&lock, b"existing lock").unwrap();
    let held_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    held_lock.try_lock().unwrap();
    let (ok, log) = cli_result(&format!("{}\n", input.display()));
    assert!(!ok);
    assert!(log.contains("运行锁"));
    assert!(!log.contains("处理成功"));
    assert_eq!(std::fs::read(&lock).unwrap(), b"existing lock");
    assert_eq!(std::fs::read_dir(&input).unwrap().count(), 0);
    drop(held_lock);
    if reports.exists() {
        std::fs::remove_dir(&reports).unwrap();
    }
    std::fs::write(&reports, b"blocked report directory").unwrap();
    let log = run_cli(&format!("{}\n", input.display()));
    assert!(log.contains("输出路径不是文件夹"));
    assert!(!log.contains("处理失败"));
    assert_eq!(
        std::fs::read(&reports).unwrap(),
        b"blocked report directory"
    );
    std::fs::remove_dir_all(base).unwrap();
}
