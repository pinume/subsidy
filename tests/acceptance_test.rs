use calamine::{Data, DataType, Range, Reader, open_workbook_auto};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use subsidy::{generate_store_finance_workbook, generate_summary_workbook};

#[path = "fixtures/cli_process.rs"]
mod cli_process;
#[path = "../src/test_support.rs"]
mod test_support;

use cli_process::run_cli_process;
use test_support::unique_temp_path;

/// 真实数据验收：数据会持续更新，只断言任意批次都成立的关系，不写死批次计数。
#[test]
#[ignore = "requires a raw dataset via COMBINE_SOURCE_DIR (any batch, e.g. ~/yz/data)"]
fn real_dataset_invariants() {
    let source_dir = std::env::var_os("COMBINE_SOURCE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set COMBINE_SOURCE_DIR to a raw dataset directory");
    let root = unique_temp_path("acceptance");
    let input = root.join("原始 数据");
    std::fs::create_dir_all(&input).unwrap();
    let mut originals = Vec::new();
    for entry in std::fs::read_dir(&source_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            std::fs::write(input.join(path.file_name().unwrap()), &bytes).unwrap();
            originals.push((path, bytes));
        }
    }
    let result = run_cli(&root);
    let log = String::from_utf8_lossy(&result.stdout);
    assert!(
        result.status.success(),
        "{log}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(log.matches("处理成功").count(), 8);
    assert_eq!(log.matches("报表成功").count(), 4);
    assert!(!log.contains("请输入选项"));

    let cleaned = root.join("国补明细");
    let height = |name: &str| {
        let mut workbook = open_workbook_auto(cleaned.join(name)).unwrap();
        assert_eq!(workbook.sheet_names().len(), 1, "{name}");
        let rows = workbook.worksheet_range_at(0).unwrap().unwrap().height() - 1;
        assert!(rows > 0, "{name} 没有明细");
        rows
    };
    for name in ["发票明细.xlsx", "收款单统计.xlsx"] {
        height(name);
    }
    let unionpay_rows = height("银联交易明细门店.xlsx");
    let refund_rows = height("回款明细家电电脑.xlsx") + height("回款明细数码.xlsx");
    let upload_rows = height("已上传数码.xlsx") + height("已上传家电电脑.xlsx");
    let coupon_rows = height("销售用券情况统计.xlsx");
    assert_eq!(std::fs::read_dir(&cleaned).unwrap().count(), 9);
    // 四类匹配互斥，合计等于销售用券明细数。
    let total: usize = log
        .split("；合计 ")
        .nth(1)
        .and_then(|rest| rest.split('。').next())
        .and_then(|n| n.parse().ok())
        .expect("匹配统计合计");
    assert_eq!(total, coupon_rows);
    assert_return_trace(&input, &cleaned);
    for (path, bytes) in originals {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "original changed: {}",
            path.display()
        );
        assert_eq!(
            std::fs::read(input.join(path.file_name().unwrap())).unwrap(),
            bytes
        );
    }

    let out_dir = root.join("国补报表");
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 4);
    let wb1_path = out_dir.join("国补上传情况汇总.xlsx");
    let wb2_path = out_dir.join("26年国补门店财务统筹表.xlsx");
    assert_ledgers(&input, &out_dir);

    // Public standalone APIs must produce the same workbook content as the shared CLI.
    let standalone = root.join("standalone");
    std::fs::create_dir(&standalone).unwrap();
    let standalone_summary = standalone.join("summary.xlsx");
    let standalone_finance = standalone.join("finance.xlsx");
    generate_summary_workbook(&cleaned, &standalone_summary).unwrap();
    generate_store_finance_workbook(&cleaned, &standalone_finance).unwrap();
    assert_workbook_equal(&wb1_path, &standalone_summary);
    assert_workbook_equal(&wb2_path, &standalone_finance);

    let mut wb1 = open_workbook_auto(&wb1_path).expect("Open WB1 failed");
    assert_eq!(
        wb1.sheet_names(),
        vec![
            "汇总",
            "品类品牌汇总",
            "审核失败明细",
            "异常回款明细",
            "异常发票明细",
            "国补发生情况"
        ]
    );

    // 汇总：第7行起依次为发生、已回款、未回款、（提示行）、通过、待审核、失败、未上传。
    let r_summary = wb1.worksheet_range("汇总").unwrap();
    let number = |row, col| {
        r_summary
            .get_value((row, col))
            .and_then(DataType::as_f64)
            .unwrap()
    };
    for row in [6, 7, 8, 10, 11, 12, 13] {
        assert!((number(row, 5) - number(row, 1) - number(row, 3)).abs() < 0.0001);
        assert_eq!(number(row, 6), number(row, 2) + number(row, 4));
    }
    assert!((number(8, 5) - (number(6, 5) - number(7, 5))).abs() < 0.0001);
    let mut coupons = open_workbook_auto(cleaned.join("销售用券情况统计.xlsx")).unwrap();
    let coupons = coupons.worksheet_range_at(0).unwrap().unwrap();
    let subsidy_total: f64 = coupons
        .rows()
        .skip(1)
        .filter_map(|row| row[5].as_f64())
        .sum();
    assert!((number(6, 5) * 10000.0 - subsidy_total).abs() < 0.01);

    assert!(wb1.worksheet_range("品类品牌汇总").unwrap().height() > 5);

    // 审核失败明细的笔数与汇总一致；家电电脑区块按商品名称排序、空值置后。
    let r_fail = wb1.worksheet_range("审核失败明细").unwrap();
    let fail_cnt = r_fail
        .rows()
        .filter(|row| row.get(2).is_some_and(|c| c == "审核失败"))
        .count();
    assert_eq!(fail_cnt as f64, number(12, 6));
    let sections = data_sections(&r_fail);
    let product_names: Vec<_> = sections[0]
        .iter()
        .map(|row| {
            let value = row[8].to_string();
            (value.is_empty(), value)
        })
        .collect();
    assert!(product_names.windows(2).all(|pair| pair[0] <= pair[1]));

    // 异常回款明细：每个区块按参考号排序，空值与重复参考号的分组规则不变。
    let r_ref_anom = wb1.worksheet_range("异常回款明细").unwrap();
    for section in data_sections(&r_ref_anom) {
        let references: Vec<_> = section
            .iter()
            .map(|row| {
                let value = row[2].to_string();
                (value.is_empty(), value)
            })
            .collect();
        let counts = references
            .iter()
            .fold(HashMap::new(), |mut counts, (_, value)| {
                *counts.entry(value).or_insert(0) += 1;
                counts
            });
        let keys: Vec<_> = references
            .iter()
            .map(|(empty, value)| (*empty, counts[value] == 1, value))
            .collect();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    let mut wb2 = open_workbook_auto(&wb2_path).expect("Open WB2 failed");
    assert_eq!(
        wb2.sheet_names(),
        vec![
            "最终匹配表（全部的国补发生数据上匹配）",
            "1.门店国补发生表（银联系统直接导出，不需要加工）",
            "2.门店累计回款表（事业部财务下发，门店筛选自己的）",
            "3.门店上传明细（从门店银联后台每月导出后汇总）"
        ]
    );
    let r_occ = wb2
        .worksheet_range("1.门店国补发生表（银联系统直接导出，不需要加工）")
        .unwrap();
    assert_eq!(r_occ.height(), unionpay_rows + 1);
    assert_eq!(r_occ.width(), 26);
    assert!(matches!(r_occ.get_value((1, 0)), Some(Data::DateTime(_))));
    let r_ref = wb2
        .worksheet_range("2.门店累计回款表（事业部财务下发，门店筛选自己的）")
        .unwrap();
    assert_eq!(r_ref.height(), refund_rows + 1);
    assert_eq!(r_ref.width(), 24);
    let r_up = wb2
        .worksheet_range("3.门店上传明细（从门店银联后台每月导出后汇总）")
        .unwrap();
    assert_eq!(r_up.height(), upload_rows + 1);
    assert_eq!(r_up.width(), 60);
    assert_eq!(r_up.get_value((0, 59)).unwrap(), "开票日期");
    let r_final = wb2
        .worksheet_range("最终匹配表（全部的国补发生数据上匹配）")
        .unwrap();
    assert_eq!(r_final.height(), unionpay_rows + 1);
    assert_eq!(r_final.width(), 27);
    assert!(matches!(r_final.get_value((1, 1)), Some(Data::DateTime(_))));
    for row in r_final.rows().skip(1) {
        let red = row[26].to_string();
        assert!(["是", "否", ""].contains(&red.as_str()), "红冲列 {red}");
        // 红冲判断依赖最终发票号。
        assert!(red.is_empty() || !row[25].to_string().is_empty());
    }

    // A failed UnionPay publication must skip finance while summary still succeeds.
    let finance_before = std::fs::read(&wb2_path).unwrap();
    let unionpay = cleaned.join("银联交易明细门店.xlsx");
    let saved_unionpay = cleaned.join("saved-unionpay.xlsx");
    std::fs::rename(&unionpay, &saved_unionpay).unwrap();
    std::fs::create_dir(&unionpay).unwrap();
    let result = run_cli(&root);
    let log = String::from_utf8_lossy(&result.stdout);
    assert!(!result.status.success());
    assert!(log.contains("成功 7，失败 1"), "{log}");
    assert!(log.contains("[国补上传情况汇总] 报表成功"), "{log}");
    assert!(log.contains("[26年国补门店财务统筹表] 跳过报表"), "{log}");
    assert_eq!(std::fs::read(&wb2_path).unwrap(), finance_before);
    std::fs::remove_dir(&unionpay).unwrap();
    std::fs::rename(&saved_unionpay, &unionpay).unwrap();

    // With a missing raw invoice, valid old cleaned/report files must remain unused.
    let invoice = cleaned.join("发票明细.xlsx");
    let invoice_before = std::fs::read(&invoice).unwrap();
    let ledgers = ["家电台账.xlsx", "数码台账.xlsx"].map(|name| out_dir.join(name));
    let reports_before: Vec<_> = [&wb1_path, &wb2_path, &ledgers[0], &ledgers[1]]
        .into_iter()
        .map(|path| (path, std::fs::read(path).unwrap()))
        .collect();
    for entry in std::fs::read_dir(&input).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("发票_") && name.ends_with(".xlsx") {
            std::fs::rename(&path, path.with_extension("xlsx.bak")).unwrap();
        }
    }
    let result = run_cli(&root);
    let log = String::from_utf8_lossy(&result.stdout);
    assert!(!result.status.success());
    assert_eq!(log.matches("跳过报表").count(), 2, "{log}");
    // 两份台账都依赖原始发票：缺少发票时均失败并保留旧文件。
    assert!(log.contains("[家电台账] 报表失败"), "{log}");
    assert!(log.contains("[数码台账] 报表失败"), "{log}");
    assert!(!log.contains("报表成功"), "{log}");
    assert!(log.contains("不使用旧文件"));
    assert_eq!(std::fs::read(&invoice).unwrap(), invoice_before);
    for (path, bytes) in reports_before {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    std::fs::remove_dir_all(root).unwrap();
}

/// 台账验收（报表规则§6）：预期值只用 calamine 从原始工作簿独立计算，不复用生产读取与补值。
fn assert_ledgers(input: &Path, out_dir: &Path) {
    let invoices = source_invoices(input);
    let mut references = None;
    for spec in &LEDGERS {
        let sources = ledger_source_rows(spec, input);
        let mut book = open_workbook_auto(out_dir.join(format!("{}.xlsx", spec.name))).unwrap();
        let output = book.worksheet_range(spec.name).unwrap();
        // 第1行标题，第2行表头，第3行起为数据。
        let rows: Vec<_> = output.rows().collect();
        let header: Vec<_> = rows[1].iter().map(text).collect();
        let rows = &rows[2..];
        assert_eq!(rows.len(), sources.len(), "{} 数据行数", spec.name);

        for (index, (source, row)) in sources.iter().zip(rows).enumerate() {
            let actual = |field: &str| {
                let col = header.iter().position(|name| name == field);
                &row[col.unwrap_or_else(|| panic!("{} 缺少输出列 {field}", spec.name))]
            };
            let at = |field: &str| {
                format!(
                    "{} 源表“{}”第{}行 → 输出第{}行 字段 {field}",
                    spec.name,
                    source.sheet,
                    source.row,
                    index + 3
                )
            };
            // 值比较会抹平单元格类型，另按实际输出单元格断言类型（§6.1 第4条）。
            for field in spec.text_columns {
                let cell = actual(field);
                assert!(
                    matches!(cell, Data::Empty | Data::String(_)),
                    "{} 应为文本：{cell:?}",
                    at(field)
                );
            }
            for field in spec.other_amount_columns {
                if let Data::String(raw) = actual(field) {
                    assert!(
                        raw.trim().parse::<Decimal>().is_err(),
                        "{} 可解析金额应为数值：{raw:?}",
                        at(field)
                    );
                }
            }
            for field in ["拨付批次", "序号", "商户订单号", spec.invoice_field] {
                assert_eq!(
                    text(actual(field)),
                    text(source.get(field)),
                    "{}",
                    at(field)
                );
            }
            assert_amount(actual("补贴金额"), source.get("补贴金额"), &at("补贴金额"));

            let number = text(source.get(spec.invoice_field)).trim().to_string();
            let invoice = invoices.get(&number);
            let red = invoice.is_some_and(|invoice| invoice.status.trim() == "已红冲");
            assert_eq!(
                text(actual("发票状态")),
                if red { "已红冲" } else { "" },
                "{}",
                at("发票状态")
            );

            let buyer = "发票头/购买方名称";
            let invoice_buyer = invoice.map_or(Data::Empty, |invoice| invoice.buyer.clone());
            if !spec.appliance {
                assert_eq!(text(actual(buyer)), text(&invoice_buyer), "{}", at(buyer));
                continue;
            }
            let expected = if blank(source.get(buyer)) {
                &invoice_buyer
            } else {
                source.get(buyer)
            };
            if blank(expected) {
                assert!(blank(actual(buyer)), "{}", at(buyer));
            } else {
                assert_eq!(text(actual(buyer)), text(expected), "{}", at(buyer));
            }
            let total = "发票金额";
            let expected = if blank(source.get(total)) {
                invoice.map_or(&Data::Empty, |invoice| &invoice.total)
            } else {
                source.get(total)
            };
            assert_amount(actual(total), expected, &at(total));

            let field = "交易参考号";
            let expected = if blank(source.get(field)) {
                let references = references.get_or_insert_with(|| source_references(input));
                let order = text(source.get("商户订单号")).trim().to_string();
                match references.get(&order) {
                    None => String::new(),
                    Some(found) => {
                        assert_eq!(
                            found.len(),
                            1,
                            "违反业务前提（同一订单号只对应一个参考号），待业务确认：订单号 {order} → {found:?}；{}",
                            at(field)
                        );
                        found.keys().next().unwrap().clone()
                    }
                }
            } else {
                text(source.get(field))
            };
            assert_eq!(text(actual(field)), expected, "{}", at(field));
        }
    }
}

struct LedgerSpec {
    name: &'static str,
    file_suffix: &'static str,
    merchant: &'static str,
    appliance: bool,
    invoice_field: &'static str,
    /// 非空时必须写为文本的标识列。
    text_columns: &'static [&'static str],
    /// 不做源值比较的金额列：可解析金额不得写成文本。
    other_amount_columns: &'static [&'static str],
    /// 本测试核对到的字段及其同义源表头（§6.2.1、§6.3.1）；首项为商户字段。
    fields: &'static [(&'static str, &'static [&'static str])],
}

const LEDGERS: [LedgerSpec; 2] = [
    LedgerSpec {
        name: "家电台账",
        file_suffix: "年家电类以旧换新补贴明细已经回款下发.xlsx",
        merchant: "89813015722APT1",
        appliance: true,
        invoice_field: "发票号",
        text_columns: &["序号", "商户订单号", "交易订单号", "交易参考号", "发票号"],
        other_amount_columns: &["其他支付", "销售金额", "实收销售金额"],
        fields: &[
            (
                "核销商编",
                &["核销商编", "商户编号", "商户号", "经销商编号"],
            ),
            ("拨付批次", &["拨付批次"]),
            ("序号", &["序号"]),
            ("商户订单号", &["商户订单号"]),
            ("补贴金额", &["补贴金额"]),
            ("发票号", &["发票号"]),
            ("发票头/购买方名称", &["发票头/购买方名称"]),
            ("发票金额", &["发票金额"]),
            ("交易参考号", &["交易参考号"]),
        ],
    },
    LedgerSpec {
        name: "数码台账",
        file_suffix: "年数码补贴明细已经回款下发.xlsx",
        merchant: "89813014812B06R",
        appliance: false,
        invoice_field: "发票号码",
        text_columns: &["序号", "商户订单号", "参考号", "发票号码"],
        other_amount_columns: &["销售金额"],
        fields: &[
            ("商户编号", &["商户编号", "经销商编号"]),
            ("拨付批次", &["拨付批次"]),
            ("序号", &["序号"]),
            ("商户订单号", &["商户订单号"]),
            ("补贴金额", &["补贴金额", "补贴销售金额"]),
            ("发票号码", &["发票号码"]),
        ],
    },
];

struct SourceRow {
    sheet: String,
    /// Excel 行号（从1开始）。
    row: u32,
    values: HashMap<&'static str, Data>,
}

impl SourceRow {
    fn get(&self, field: &str) -> &Data {
        &self.values[field]
    }
}

/// 按 Excel 绝对行（从0开始）返回从 A 列起的整行。
fn absolute_row(range: &Range<Data>, row: u32) -> Vec<Data> {
    let end = range.end().map_or(0, |(_, col)| col + 1);
    (0..end)
        .map(|col| range.get_value((row, col)).cloned().unwrap_or(Data::Empty))
        .collect()
}

fn header_names(range: &Range<Data>, row: u32) -> Vec<String> {
    absolute_row(range, row)
        .iter()
        .map(|cell| text(cell).trim().to_string())
        .collect()
}

fn column(header: &[String], names: &[&str], location: &str) -> Option<usize> {
    let found: Vec<_> = (0..header.len())
        .filter(|&col| names.contains(&header[col].as_str()))
        .collect();
    assert!(found.len() <= 1, "{location} 表头 {names:?} 重复");
    found.first().copied()
}

fn files_matching(input: &Path, matches: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(input)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| matches(&path.file_name().unwrap().to_string_lossy()))
        .collect();
    files.sort();
    files
}

/// 最新年份源文件中的本店记录：按工作表及行顺序读取，再按拨付批次首次出现的顺序稳定分组。
fn ledger_source_rows(spec: &LedgerSpec, input: &Path) -> Vec<SourceRow> {
    let path = files_matching(input, |name| {
        name.strip_suffix(spec.file_suffix)
            .is_some_and(|year| year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()))
    })
    .pop()
    .unwrap_or_else(|| panic!("{} 缺少补贴明细源文件", spec.name));
    let mut book = open_workbook_auto(path).unwrap();
    let mut groups: Vec<(String, Vec<SourceRow>)> = Vec::new();
    for sheet in book.sheet_names() {
        if sheet.trim() == "汇总" {
            continue;
        }
        let range = book.worksheet_range(&sheet).unwrap();
        let find_header = |names: &[&str]| {
            (0..20).find(|&row| {
                header_names(&range, row)
                    .iter()
                    .any(|name| names.contains(&name.as_str()))
            })
        };
        let header_row = find_header(&["拨付批次"])
            .or_else(|| find_header(spec.fields[0].1))
            .unwrap_or_else(|| panic!("{} / {sheet} 找不到表头", spec.name));
        let header = header_names(&range, header_row);
        let columns: Vec<_> = spec
            .fields
            .iter()
            .map(|(field, names)| (*field, column(&header, names, &sheet)))
            .collect();
        let merchant = columns[0].1.expect("源表应有商户字段");
        for row in header_row + 1..=range.end().map_or(0, |(row, _)| row) {
            let cells = absolute_row(&range, row);
            if text(&cells[merchant]).trim() != spec.merchant {
                continue;
            }
            let source = SourceRow {
                sheet: sheet.clone(),
                row: row + 1,
                values: columns
                    .iter()
                    .map(|(field, col)| (*field, col.map_or(Data::Empty, |col| cells[col].clone())))
                    .collect(),
            };
            let batch = text(source.get("拨付批次")).trim().to_string();
            match groups.iter_mut().find(|(name, _)| *name == batch) {
                Some((_, rows)) => rows.push(source),
                None => groups.push((batch, vec![source])),
            }
        }
    }
    groups.into_iter().flat_map(|(_, rows)| rows).collect()
}

struct SourceInvoice {
    status: String,
    buyer: Data,
    total: Data,
}

/// 最新日期发票：按去首尾空白的非空数电发票号码索引。
fn source_invoices(input: &Path) -> HashMap<String, SourceInvoice> {
    let path = files_matching(input, |name| {
        name.strip_prefix("发票_")
            .and_then(|rest| rest.strip_suffix(".xlsx"))
            .is_some_and(|date| date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
    })
    .pop()
    .expect("缺少原始发票");
    let mut book = open_workbook_auto(path).unwrap();
    let range = book.worksheet_range_at(0).unwrap().unwrap();
    let header = header_names(&range, 5);
    let [number, buyer, total, status] = ["数电发票号码", "购方名称", "合计含税金额", "开票状态"]
        .map(|name| column(&header, &[name], "原始发票").expect(name));
    let mut invoices = HashMap::new();
    for row in 6..=range.end().unwrap().0 {
        let cells = absolute_row(&range, row);
        let key = text(&cells[number]).trim().to_string();
        if key.is_empty() {
            continue;
        }
        let invoice = SourceInvoice {
            status: text(&cells[status]),
            buyer: cells[buyer].clone(),
            total: cells[total].clone(),
        };
        assert!(
            invoices.insert(key.clone(), invoice).is_none(),
            "原始发票第{}行号码重复：{key}",
            row + 1
        );
    }
    invoices
}

/// 家电已上传原始工作簿：订单号 → 各不同非空检索参考号及其首个源位置。
fn source_references(input: &Path) -> HashMap<String, BTreeMap<String, String>> {
    let mut references: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for path in files_matching(input, |name| {
        name.starts_with("MER_89813015722APT1_") && name.ends_with(".xlsx")
    }) {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut book = open_workbook_auto(&path).unwrap();
        for sheet in book.sheet_names() {
            let range = book.worksheet_range(&sheet).unwrap();
            let header = header_names(&range, 1);
            let [order, reference] = ["订单号", "检索参考号"].map(|name| {
                header
                    .iter()
                    .position(|found| found == name)
                    .unwrap_or_else(|| panic!("{file} / {sheet} 缺少 {name}"))
            });
            for row in 2..=range.end().map_or(0, |(row, _)| row) {
                let cells = absolute_row(&range, row);
                let key = text(&cells[order]).trim().to_string();
                let value = text(&cells[reference]);
                if !key.is_empty() && !value.trim().is_empty() {
                    references
                        .entry(key)
                        .or_default()
                        .entry(value)
                        .or_insert_with(|| format!("{file} / {sheet} 第{}行", row + 1));
                }
            }
        }
    }
    references
}

/// 标识与序号按文本比较；数值型单元格还原为完整整数文本。
fn text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::Float(value) if value.fract() == 0.0 => format!("{value:.0}"),
        other => other.to_string(),
    }
}

fn blank(cell: &Data) -> bool {
    text(cell).trim().is_empty()
}

/// 金额按十进制数值比较；空值为 `Ok(None)`（数值零是已有金额），无法解析时按原文比较。
fn amount(cell: &Data) -> Result<Option<Decimal>, String> {
    if blank(cell) {
        return Ok(None);
    }
    match cell {
        Data::Float(value) => Ok(Decimal::from_f64(*value)),
        Data::Int(value) => Ok(Some(Decimal::from(*value))),
        other => {
            let raw = other.to_string();
            raw.trim().parse().map(Some).map_err(|_| raw)
        }
    }
}

/// 可解析的源金额须输出为等值数值单元格；无法解析的按原文保留；空值留空。
fn assert_amount(actual: &Data, expected: &Data, location: &str) {
    match amount(expected) {
        Ok(None) => assert!(blank(actual), "{location} 应留空：{actual:?}"),
        Ok(value) => {
            assert!(
                matches!(actual, Data::Float(_) | Data::Int(_)),
                "{location} 应为数值：{actual:?}"
            );
            assert_eq!(amount(actual), Ok(value), "{location}");
        }
        Err(raw) => assert_eq!(actual, &Data::String(raw), "{location} 应保留原文"),
    }
}

/// 报表工作表的数据区块：第6行起，区块之间隔一空行、区块标题和列标题共3行。
fn data_sections(range: &Range<Data>) -> Vec<Vec<&[Data]>> {
    let rows: Vec<_> = range.rows().collect();
    let mut sections = Vec::new();
    let mut index = 5;
    while index < rows.len() {
        let mut section = Vec::new();
        while index < rows.len() && rows[index].iter().any(|cell| !cell.is_empty()) {
            section.push(rows[index]);
            index += 1;
        }
        sections.push(section);
        index += 3;
    }
    sections
}

fn run_cli(root: &std::path::Path) -> std::process::Output {
    run_cli_process(Some(root), "  \"原始 数据\"  \n".as_bytes())
}

fn assert_return_trace(input: &std::path::Path, cleaned: &std::path::Path) {
    let mut source = open_workbook_auto(input.join("收款单统计.xlsx")).unwrap();
    let source = source.worksheet_range_at(0).unwrap().unwrap();
    let mut receipts = open_workbook_auto(cleaned.join("收款单统计.xlsx")).unwrap();
    let receipts = receipts.worksheet_range_at(0).unwrap().unwrap();
    let rows: Vec<_> = receipts.rows().skip(1).collect();
    // 源表列序可能变化，与生产代码一样按表头名称定位。
    let category = source
        .rows()
        .nth(1)
        .and_then(|header| header.iter().position(|cell| *cell == "销售类别"))
        .expect("收款单源表第2行应有销售类别");
    let categories: Vec<_> = source
        .rows()
        .skip(2)
        .take(rows.len())
        .map(|row| row[category].to_string())
        .collect();
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let original = row[3].to_string();
        if !original.is_empty() {
            children.entry(original).or_default().push(index);
        }
    }
    for (index, row) in rows.iter().enumerate() {
        // 独立验算：从正常销售单向后找补差/换货及实际退单，与生产追溯方向相反。
        let mut returned = false;
        if categories[index] == "正常销售" {
            let mut pending = vec![row[4].to_string()];
            let mut seen = HashSet::new();
            while let Some(key) = pending.pop() {
                if key.is_empty() || !seen.insert(key.clone()) {
                    continue;
                }
                for &child in children.get(&key).into_iter().flatten() {
                    match categories[child].as_str() {
                        "退货" => returned = true,
                        "零售补差" | "同型号换货" => {
                            pending.push(rows[child][4].to_string())
                        }
                        _ => {}
                    }
                }
            }
        }
        let expected = if categories[index] == "退货" {
            "退货-退单"
        } else if returned {
            "退货-原单"
        } else {
            ""
        };
        assert_eq!(row[5].to_string(), expected, "收款单第{}行", index + 2);
    }
}

fn assert_workbook_equal(left: &std::path::Path, right: &std::path::Path) {
    let mut left = zip::ZipArchive::new(std::fs::File::open(left).unwrap()).unwrap();
    let mut right = zip::ZipArchive::new(std::fs::File::open(right).unwrap()).unwrap();
    let names: Vec<_> = left.file_names().map(str::to_owned).collect();
    assert_eq!(left.len(), right.len());
    for name in names {
        // Creation timestamps do not affect workbook values or formatting.
        if name == "docProps/core.xml" {
            continue;
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        left.by_name(&name).unwrap().read_to_end(&mut a).unwrap();
        right.by_name(&name).unwrap().read_to_end(&mut b).unwrap();
        assert!(a == b, "workbook entry differs: {name}");
    }
}
