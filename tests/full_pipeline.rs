use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use calamine::{Data, Reader, open_workbook_auto};
use rust_xlsxwriter::Workbook;

#[path = "fixtures/headers.rs"]
mod headers;

fn export(
    input: &Path,
    name: &str,
    header: &str,
    header_row: u32,
    title: &str,
    rows: &[&[(&str, &str)]],
    footer: Option<&str>,
) {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    if header_row > 0 {
        sheet.write_string(0, 0, title).unwrap();
    }
    let fields: Vec<_> = header.split('|').collect();
    for (col, field) in fields.iter().enumerate() {
        sheet.write_string(header_row, col as u16, *field).unwrap();
    }
    for (index, values) in rows.iter().enumerate() {
        for (field, value) in *values {
            let col = fields.iter().position(|name| name == field).unwrap();
            sheet
                .write_string(header_row + 1 + index as u32, col as u16, *value)
                .unwrap();
        }
    }
    if let Some(footer) = footer {
        sheet
            .write_string(header_row + 1 + rows.len() as u32, 0, footer)
            .unwrap();
    }
    book.save(input.join(name)).unwrap();
}

fn field(path: &Path, sheet: usize, row: usize, name: &str) -> Data {
    let mut book = open_workbook_auto(path).unwrap();
    let range = book.worksheet_range_at(sheet).unwrap().unwrap();
    let header = range.rows().next().unwrap();
    let col = header.iter().position(|cell| cell == name).unwrap();
    range.get((row, col)).unwrap().clone()
}

#[test]
fn cli_cleans_all_eight_categories_and_publishes_four_excel_reports() {
    let root = std::env::temp_dir().join(format!(
        "subsidy-full-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let input = root.join("raw");
    std::fs::create_dir_all(&input).unwrap();
    export(
        &input,
        "发票_20260914.xlsx",
        headers::INVOICE,
        5,
        "发票",
        &[&[
            ("开票时间", "2026-09-14 10:00:00"),
            ("开票类型", "蓝票"),
            ("数电发票号码", "123"),
            ("主要商品名称", "海尔洗衣机"),
            ("备注信息", "销售日期2026-09-14 单据号ZFFX000001"),
            ("开票状态", "开票完成"),
        ]],
        None,
    );
    export(
        &input,
        "收款单统计.xlsx",
        headers::RECEIPTS,
        1,
        "标题",
        &[&[
            ("日期", "2026-09-14"),
            ("单据号", "收款ZFFX000002"),
            ("销售类别", "退货"),
        ]],
        Some("合计"),
    );
    export(
        &input,
        "销售用券情况统计.xlsx",
        headers::COUPONS,
        1,
        "销售用券情况统计",
        &[
            &[
                ("单据号", "收款ZFFX000001"),
                ("单据日期", "2026-09-14"),
                ("商品名称", "海尔洗衣机"),
                ("品牌", "海尔"),
                ("财务大类", "洗衣机"),
                ("明细摘要", "16867252734N"),
                ("合计", "150.00"),
            ],
            &[
                ("单据号", "收款ZFFX000002"),
                ("单据日期", "2026-09-14"),
                ("商品名称", "手机"),
                ("品牌", "华为"),
                ("财务大类", "新业务类"),
                ("合计", "-15.00"),
            ],
        ],
        Some("合计"),
    );
    export(
        &input,
        "89813015722APT1_MX_20260914101809_1.xlsx",
        headers::UNIONPAY,
        1,
        "汇总",
        &[&[
            ("清算时间", "20260914"),
            ("交易时间", "2026-09-14 10:18:09"),
            ("交易类型", "消费"),
            ("交易金额", "1000.00"),
            ("检索号", "16867252734N"),
            ("商户号", "89813015722APT1"),
            ("商户订单号", "order-a"),
        ]],
        Some("请注意：D1手续费字段为预估数据仅供参考，实际以17：40分之后的D1划付数据为准。"),
    );
    for (name, header, merchant_field, time_field, merchant, order, subsidy) in [
        (
            "2026年以旧换新补贴明细.xlsx",
            headers::REFUND,
            "核销商编",
            "交易完成时间",
            "89813015722APT1",
            "order-a",
            "150.00",
        ),
        (
            "2026年数码补贴明细.xlsx",
            headers::DIGITAL_REFUND,
            "商户编号",
            "提交时间",
            "89813014812B06R",
            "other",
            "10.00",
        ),
    ] {
        export(
            &input,
            name,
            header,
            0,
            "",
            &[&[
                ("拨付批次", "批次1"),
                (merchant_field, merchant),
                ("商户订单号", order),
                ("补贴金额", subsidy),
                (time_field, "2026-09-14 10:00:00"),
            ]],
            None,
        );
    }
    for (merchant, tail, uuid, order, reference, status, invoice) in [
        (
            "89813015722APT1",
            headers::APPLIANCE,
            "a",
            "order-a",
            "16867252734N",
            "待审核",
            "123",
        ),
        (
            "89813014812B06R",
            headers::DIGITAL,
            "d",
            "order-d",
            "digital-ref",
            "审核终止",
            "",
        ),
    ] {
        export(
            &input,
            &format!("MER_{merchant}_20260914101809_yjhx.xlsx"),
            &format!("{}|{tail}", headers::UPLOADED),
            1,
            "标题",
            &[&[
                ("实时清分UUID", uuid),
                ("商户号", merchant),
                ("订单号", order),
                ("检索参考号", reference),
                ("状态", status),
                ("发票号码", invoice),
                ("交易金额", "1000.00"),
                ("交易日期", "20260914"),
            ]],
            None,
        );
    }
    let originals: Vec<_> = std::fs::read_dir(&input)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(cli.stdin.take().unwrap(), "{}", input.display()).unwrap();
    let result = cli.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let cleaned = root.join("国补明细");
    assert_eq!(
        std::fs::read_dir(&cleaned)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "xlsx"))
            .count(),
        8
    );
    assert_eq!(
        field(&cleaned.join("已上传家电电脑.xlsx"), 0, 1, "状态"),
        "已回款"
    );
    assert_eq!(
        field(&cleaned.join("已上传家电电脑.xlsx"), 0, 1, "补贴金额"),
        Data::Float(150.0)
    );
    assert_eq!(
        field(&cleaned.join("已上传数码.xlsx"), 0, 1, "状态"),
        "审核终止"
    );
    assert_eq!(
        field(&cleaned.join("销售用券情况统计.xlsx"), 0, 1, "数电发票号码"),
        "123"
    );
    assert_eq!(
        field(&cleaned.join("销售用券情况统计.xlsx"), 0, 1, "备注"),
        "已回款"
    );
    assert_eq!(
        field(&cleaned.join("销售用券情况统计.xlsx"), 0, 2, "备注"),
        "退货-退单"
    );
    let reports = root.join("国补报表");
    assert_eq!(std::fs::read_dir(&reports).unwrap().count(), 4);
    for (name, order) in [("家电台账", "order-a"), ("数码台账", "other")] {
        let mut book = open_workbook_auto(reports.join(format!("{name}.xlsx"))).unwrap();
        assert_eq!(book.sheet_names(), [name]);
        let range = book.worksheet_range(name).unwrap();
        let rows: Vec<_> = range.rows().collect();
        assert_eq!(rows.len(), 3, "{name}: 标题、表头及1条本店记录");
        assert_eq!(rows[1].len(), 26);
        let col = |field: &str| rows[1].iter().position(|cell| cell == field).unwrap();
        assert_eq!(rows[2][col("序号")], "1-01");
        assert_eq!(rows[2][col("商户订单号")], order);
    }
    assert_eq!(
        open_workbook_auto(reports.join("国补上传情况汇总.xlsx"))
            .unwrap()
            .sheet_names()
            .len(),
        5
    );
    let finance = reports.join("26年国补门店财务统筹表.xlsx");
    assert_eq!(open_workbook_auto(&finance).unwrap().sheet_names().len(), 4);
    assert_eq!(field(&finance, 0, 1, "状态"), "已回款");
    assert_eq!(field(&finance, 0, 1, "发票号"), "123");
    assert_eq!(field(&finance, 0, 1, "品牌"), "海尔");
    assert_eq!(field(&finance, 0, 1, "交易金额"), Data::Float(1000.0));
    for (path, bytes) in originals {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_warns_on_duplicate_invoice_keys_and_only_the_ledgers_fail() {
    let root = std::env::temp_dir().join(format!(
        "subsidy-conflict-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let input = root.join("raw");
    std::fs::create_dir_all(&input).unwrap();
    // Two conflicting invoice records with the same number but differing product names
    export(
        &input,
        "发票_20260914.xlsx",
        headers::INVOICE,
        5,
        "发票",
        &[
            &[
                ("开票时间", "2026-09-14 10:00:00"),
                ("开票类型", "蓝票"),
                ("数电发票号码", "123"),
                ("主要商品名称", "海尔洗衣机"),
                ("备注信息", "销售日期2026-09-14 单据号ZFFX000001"),
                ("开票状态", "开票完成"),
            ],
            &[
                ("开票时间", "2026-09-14 10:05:00"),
                ("开票类型", "蓝票"),
                ("数电发票号码", "123"),
                ("主要商品名称", "美的空调"),
                ("备注信息", "销售日期2026-09-14 单据号ZFFX000001"),
                ("开票状态", "开票完成"),
            ],
        ],
        None,
    );
    export(
        &input,
        "收款单统计.xlsx",
        headers::RECEIPTS,
        1,
        "标题",
        &[&[
            ("日期", "2026-09-14"),
            ("单据号", "收款ZFFX000002"),
            ("销售类别", "退货"),
        ]],
        Some("合计"),
    );
    export(
        &input,
        "销售用券情况统计.xlsx",
        headers::COUPONS,
        1,
        "销售用券情况统计",
        &[
            &[
                ("单据号", "收款ZFFX000001"),
                ("单据日期", "2026-09-14"),
                ("商品名称", "海尔洗衣机"),
                ("品牌", "海尔"),
                ("财务大类", "洗衣机"),
                ("明细摘要", "16867252734N"),
                ("合计", "150.00"),
            ],
            &[
                ("单据号", "收款ZFFX000002"),
                ("单据日期", "2026-09-14"),
                ("商品名称", "手机"),
                ("品牌", "华为"),
                ("财务大类", "新业务类"),
                ("合计", "-15.00"),
            ],
        ],
        Some("合计"),
    );
    export(
        &input,
        "89813015722APT1_MX_20260914101809_1.xlsx",
        headers::UNIONPAY,
        1,
        "汇总",
        &[&[
            ("清算时间", "20260914"),
            ("交易时间", "2026-09-14 10:18:09"),
            ("交易类型", "消费"),
            ("交易金额", "1000.00"),
            ("检索号", "16867252734N"),
            ("商户号", "89813015722APT1"),
            ("商户订单号", "order-a"),
        ]],
        Some("请注意：D1手续费字段为预估数据仅供参考，实际以17：40分之后的D1划付数据为准。"),
    );
    for (name, header, merchant_field, time_field, merchant, order, subsidy) in [
        (
            "2026年以旧换新补贴明细.xlsx",
            headers::REFUND,
            "核销商编",
            "交易完成时间",
            "89813015722APT1",
            "order-a",
            "150.00",
        ),
        (
            "2026年数码补贴明细.xlsx",
            headers::DIGITAL_REFUND,
            "商户编号",
            "提交时间",
            "89813014812B06R",
            "other",
            "10.00",
        ),
    ] {
        export(
            &input,
            name,
            header,
            0,
            "",
            &[&[
                ("拨付批次", "批次1"),
                (merchant_field, merchant),
                ("商户订单号", order),
                ("补贴金额", subsidy),
                (time_field, "2026-09-14 10:00:00"),
            ]],
            None,
        );
    }
    for (merchant, tail, uuid, order, reference, status, invoice) in [
        (
            "89813015722APT1",
            headers::APPLIANCE,
            "a",
            "order-a",
            "16867252734N",
            "待审核",
            "123",
        ),
        (
            "89813014812B06R",
            headers::DIGITAL,
            "d",
            "order-d",
            "digital-ref",
            "审核终止",
            "",
        ),
    ] {
        export(
            &input,
            &format!("MER_{merchant}_20260914101809_yjhx.xlsx"),
            &format!("{}|{tail}", headers::UPLOADED),
            1,
            "标题",
            &[&[
                ("实时清分UUID", uuid),
                ("商户号", merchant),
                ("订单号", order),
                ("检索参考号", reference),
                ("状态", status),
                ("发票号码", invoice),
                ("交易金额", "1000.00"),
                ("交易日期", "20260914"),
            ]],
            None,
        );
    }

    let mut cli = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(cli.stdin.take().unwrap(), "{}", input.display()).unwrap();
    let result = cli.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    // 原有报表只告警；台账按唯一键使用数电发票号码，重复即失败，整体退出码为 1。
    assert_eq!(
        result.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout
            .contains("[家电台账] 报表失败：读取失败：发票数电发票号码重复，无法唯一补充台账：123"),
        "Appliance ledger should reject duplicate invoice numbers: {stdout}"
    );
    assert!(
        stdout
            .contains("[数码台账] 报表失败：读取失败：发票数电发票号码重复，无法唯一补充台账：123"),
        "Digital ledger should reject duplicate invoice numbers: {stdout}"
    );

    // Assert warnings are surfaced to CLI output with proper format
    assert!(
        stdout.contains("[国补上传情况汇总] 注意：发票商品名称索引：1 个键对应多条内容不同的记录，已按未命中处理（123）"),
        "Missing summary warning in stdout: {stdout}"
    );
    assert!(
        stdout.contains("[26年国补门店财务统筹表] 注意：发票明细索引：1 个键对应多条内容不同的记录，已按未命中处理（123）"),
        "Missing finance warning in stdout: {stdout}"
    );
    assert!(
        stdout.contains("[国补上传情况汇总] 报表成功。"),
        "Summary report should report success: {stdout}"
    );
    assert!(
        stdout.contains("[26年国补门店财务统筹表] 报表成功。"),
        "Finance report should report success: {stdout}"
    );

    let reports = root.join("国补报表");
    assert_eq!(std::fs::read_dir(&reports).unwrap().count(), 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_ledger_keeps_records_when_headers_start_in_column_b() {
    let root = std::env::temp_dir().join(format!("ledger-offset-{}", std::process::id()));
    let input = root.join("raw");
    std::fs::create_dir_all(&input).unwrap();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.write_string(0, 1, "商户编号").unwrap();
    sheet.write_string(0, 2, "商户订单号").unwrap();
    sheet.write_string(1, 1, "89813014812B06R").unwrap();
    sheet.write_string(1, 2, "ORDER-1").unwrap();
    book.save(input.join("2026年数码补贴明细.xlsx")).unwrap();
    export(
        &input,
        "发票_20261109.xlsx",
        headers::INVOICE,
        5,
        "",
        &[],
        None,
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", input.display()).unwrap();
    let result = child.wait_with_output().unwrap();
    // Other required sources are absent; the independent digital ledger must still publish.
    assert_eq!(result.status.code(), Some(1));
    let mut book = open_workbook_auto(root.join("国补报表/数码台账.xlsx")).unwrap();
    let range = book.worksheet_range_at(0).unwrap().unwrap();
    assert_eq!(range.height(), 3);
    assert!(
        range
            .rows()
            .nth(2)
            .unwrap()
            .iter()
            .any(|cell| cell == "ORDER-1")
    );
    std::fs::remove_dir_all(root).unwrap();
}
