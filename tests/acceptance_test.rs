use calamine::{Data, DataType, Range, Reader, open_workbook_auto};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use subsidy::{generate_store_finance_workbook, generate_summary_workbook};

/// 真实数据验收：数据会持续更新，只断言任意批次都成立的关系，不写死批次计数。
#[test]
#[ignore = "requires a raw dataset via COMBINE_SOURCE_DIR (any batch, e.g. ~/yz/data)"]
fn real_dataset_invariants() {
    let source_dir = std::env::var_os("COMBINE_SOURCE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set COMBINE_SOURCE_DIR to a raw dataset directory");
    let root = std::env::temp_dir().join(format!(
        "combine-acceptance-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
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
    assert_eq!(log.matches("报表成功").count(), 2);
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
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 2);
    let wb1_path = out_dir.join("国补上传情况汇总.xlsx");
    let wb2_path = out_dir.join("26年国补门店财务统筹表.xlsx");

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
            "异常发票明细"
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
    let reports_before: Vec<_> = [&wb1_path, &wb2_path]
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
    assert!(!log.contains("报表成功"));
    assert!(log.contains("不使用旧文件"));
    assert_eq!(std::fs::read(&invoice).unwrap(), invoice_before);
    for (path, bytes) in reports_before {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    std::fs::remove_dir_all(root).unwrap();
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_subsidy"))
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "  \"原始 数据\"  ").unwrap();
    child.wait_with_output().unwrap()
}

fn assert_return_trace(input: &std::path::Path, cleaned: &std::path::Path) {
    let mut source = open_workbook_auto(input.join("收款单统计.xlsx")).unwrap();
    let source = source.worksheet_range_at(0).unwrap().unwrap();
    let mut receipts = open_workbook_auto(cleaned.join("收款单统计.xlsx")).unwrap();
    let receipts = receipts.worksheet_range_at(0).unwrap().unwrap();
    let rows: Vec<_> = receipts.rows().skip(1).collect();
    let categories: Vec<_> = source
        .rows()
        .skip(2)
        .take(rows.len())
        .map(|row| row[8].to_string())
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
