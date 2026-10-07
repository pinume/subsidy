use calamine::{Data, DataType, Reader, open_workbook_auto};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use subsidy::{generate_store_finance_workbook, generate_summary_workbook};

#[test]
#[ignore = "requires the 2026-10-01 raw dataset via COMBINE_SOURCE_DIR"]
fn test_all_acceptance_benchmarks() {
    let source_dir = std::env::var_os("COMBINE_SOURCE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set COMBINE_SOURCE_DIR to the 2026-10-01 raw dataset");
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
    for (name, expected) in [
        ("发票明细.xlsx", 16170),
        ("已上传数码.xlsx", 7540),
        ("已上传家电电脑.xlsx", 5134),
        ("银联交易明细门店.xlsx", 14552),
        ("回款明细家电电脑.xlsx", 4272),
        ("回款明细数码.xlsx", 6391),
        ("收款单统计.xlsx", 21522),
        ("销售用券情况统计.xlsx", 17384),
    ] {
        let mut workbook = open_workbook_auto(cleaned.join(name)).unwrap();
        assert_eq!(workbook.sheet_names().len(), 1);
        let rows = workbook.worksheet_range_at(0).unwrap().unwrap();
        assert_eq!(rows.height() - 1, expected, "{name}");
    }
    assert_eq!(std::fs::read_dir(&cleaned).unwrap().count(), 9);
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
    assert!(!out_dir.join("国补上传情况汇总.md").exists());
    assert!(!standalone_summary.with_extension("md").exists());

    let mut wb1 = open_workbook_auto(&wb1_path).expect("Open WB1 failed");
    let sheets1 = wb1.sheet_names();
    assert_eq!(
        sheets1,
        vec![
            "汇总",
            "品类品牌汇总",
            "审核失败明细",
            "异常回款明细",
            "异常发票明细"
        ]
    );

    let r_summary = wb1.worksheet_range("汇总").unwrap();
    let number = |row, col| {
        r_summary
            .get_value((row, col))
            .and_then(DataType::as_f64)
            .unwrap()
    };
    assert!((number(6, 1) - 357.08).abs() < 0.01);
    assert!((number(6, 3) - 323.31).abs() < 0.01);
    assert!((number(6, 5) - 680.39).abs() < 0.01);
    assert!((number(7, 5) - 506.78).abs() < 0.01);
    assert!((number(8, 5) - 173.61).abs() < 0.01);
    assert!((number(20, 6) - 0.8514).abs() < 0.0001);

    // Verify 品类品牌汇总 has 44 brand rows (Rows 6-49)
    let r_cat = wb1.worksheet_range("品类品牌汇总").unwrap();
    assert_eq!(r_cat.rows().count(), 49); // Row 1 to 49

    // Verify 审核失败明细 has 81 appliance + 79 digital = 160 (including 审核终止)
    let r_fail = wb1.worksheet_range("审核失败明细").unwrap();
    // Count rows with status == "审核失败"
    let fail_cnt = r_fail
        .rows()
        .filter(|r| r.get(2).map(|c| c == "审核失败").unwrap_or(false))
        .count();
    assert_eq!(fail_cnt, 160);
    let product_names: Vec<_> = r_fail
        .rows()
        .skip(5)
        .take(81)
        .map(|row| {
            let value = row[8].to_string();
            (value.is_empty(), value)
        })
        .collect();
    assert!(product_names.windows(2).all(|pair| pair[0] <= pair[1]));

    // Verify 异常回款明细 has 56 + 2 = 58 rows (Row 6-61 and Row 65-66)
    let r_ref_anom = wb1.worksheet_range("异常回款明细").unwrap();
    let ref_anom_cnt = r_ref_anom
        .rows()
        .enumerate()
        .filter(|(idx, _)| {
            let row_num = idx + 1;
            (6..=61).contains(&row_num) || (65..=66).contains(&row_num)
        })
        .count();
    assert_eq!(ref_anom_cnt, 58);
    for references in [
        r_ref_anom
            .rows()
            .skip(5)
            .take(56)
            .map(|row| {
                let value = row[2].to_string();
                (value.is_empty(), value)
            })
            .collect::<Vec<_>>(),
        r_ref_anom
            .rows()
            .skip(64)
            .take(2)
            .map(|row| {
                let value = row[2].to_string();
                (value.is_empty(), value)
            })
            .collect::<Vec<_>>(),
    ] {
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

    // Verify 异常发票明细 has 32 rows (Row 6-37)
    let r_inv_anom = wb1.worksheet_range("异常发票明细").unwrap();
    let inv_anom_cnt = r_inv_anom
        .rows()
        .enumerate()
        .filter(|(idx, _)| {
            let row_num = idx + 1;
            (6..=37).contains(&row_num)
        })
        .count();
    assert_eq!(inv_anom_cnt, 32);

    // 2. Run Workbook 2
    assert!(wb2_path.exists(), "WB2 file should exist");

    let mut wb2 = open_workbook_auto(&wb2_path).expect("Open WB2 failed");
    let sheets2 = wb2.sheet_names();
    assert_eq!(
        sheets2,
        vec![
            "最终匹配表（全部的国补发生数据上匹配）",
            "1.门店国补发生表（银联系统直接导出，不需要加工）",
            "2.门店累计回款表（事业部财务下发，门店筛选自己的）",
            "3.门店上传明细（从门店银联后台每月导出后汇总）"
        ]
    );

    // Verify Sheet 2: 1.门店国补发生表 (14,552 data rows + 1 header = 14,553)
    let r_occ = wb2
        .worksheet_range("1.门店国补发生表（银联系统直接导出，不需要加工）")
        .unwrap();
    assert_eq!(r_occ.rows().count(), 14553);
    assert_eq!(r_occ.rows().next().unwrap().len(), 26);
    assert!(matches!(r_occ.get_value((1, 0)), Some(Data::DateTime(_))));

    // Verify Sheet 3: 2.门店累计回款表 (10,663 data rows + 1 header = 10,664)
    let r_ref = wb2
        .worksheet_range("2.门店累计回款表（事业部财务下发，门店筛选自己的）")
        .unwrap();
    assert_eq!(r_ref.rows().count(), 10664);
    assert_eq!(r_ref.rows().next().unwrap().len(), 24);

    // Verify Sheet 4: 3.门店上传明细 (12,674 data rows + 1 header = 12,675)
    let r_up = wb2
        .worksheet_range("3.门店上传明细（从门店银联后台每月导出后汇总）")
        .unwrap();
    assert_eq!(r_up.rows().count(), 12675);
    assert_eq!(r_up.rows().next().unwrap().len(), 60);
    assert_eq!(r_up.rows().next().unwrap()[59], "开票日期");

    // Verify Sheet 1: 最终匹配表
    let r_final = wb2
        .worksheet_range("最终匹配表（全部的国补发生数据上匹配）")
        .unwrap();
    assert_eq!(r_final.rows().count(), 14553); // 14,552 data rows + 1 header
    assert_eq!(r_final.rows().next().unwrap().len(), 27);
    assert!(matches!(r_final.get_value((1, 1)), Some(Data::DateTime(_))));

    let mut ret_cnt = 0;
    let mut unsubmitted_cnt = 0;
    let mut invoice_cnt = 0;
    let mut red_yes_cnt = 0;
    let mut red_no_cnt = 0;
    let mut red_blank_cnt = 0;

    for row in r_final.rows().skip(1) {
        let status = row[24].to_string();
        if status == "已退货" {
            ret_cnt += 1;
        } else if status == "未提交" {
            unsubmitted_cnt += 1;
        }

        let inv_no = row[25].to_string();
        if !inv_no.is_empty() {
            invoice_cnt += 1;
        }

        let red = row[26].to_string();
        if red == "是" {
            red_yes_cnt += 1;
        } else if red == "否" {
            red_no_cnt += 1;
        } else {
            red_blank_cnt += 1;
        }
    }

    assert_eq!(ret_cnt, 731, "已退货单据应为 731 笔");
    assert_eq!(unsubmitted_cnt, 1166, "未提交单据应为 1166 笔");
    assert_eq!(invoice_cnt, 12687, "有发票号单据应为 12687 笔");
    assert_eq!(red_yes_cnt, 70, "红冲为'是'应为 70 笔");
    assert_eq!(red_no_cnt, 12617, "红冲为'否'应为 12617 笔");
    assert_eq!(red_blank_cnt, 1865, "红冲留空应为 1865 笔");

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
    std::fs::rename(
        input.join("发票_20261001.xlsx"),
        input.join("发票_20261001.xlsx.bak"),
    )
    .unwrap();
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
    println!("All acceptance benchmarks passed 100%!");
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
    // 已确认仅补差/换货的11笔原销售，不再进入退货集合。
    let only_exchanges = [
        "260619ZEXQ000026",
        "260314ZEXQ000047",
        "260816ZFEG000033",
        "260107ZFP3000067",
        "260606ZGI8000004",
        "260620ZGI8000022",
        "260605ZGRK000003",
        "260307ZH3X000004",
        "260315ZH6H000029",
        "260719ZHOH000027",
        "260822ZHOH000028",
    ];
    for key in only_exchanges {
        let matches: Vec<_> = rows.iter().filter(|row| row[4] == key).collect();
        assert!(!matches.is_empty(), "{key}");
        assert!(
            matches.iter().all(|row| row[5].to_string().is_empty()),
            "{key}"
        );
    }
    // 实际换货后再退货的链：原销售单仍标为退货，换货单始终留空。
    for (key, expected) in [
        ("260426ZH3X000025", "退货-原单"),
        ("2605030233000049", ""),
        ("2605050233000077", "退货-退单"),
    ] {
        let matches: Vec<_> = rows.iter().filter(|row| row[4] == key).collect();
        assert!(!matches.is_empty(), "{key}");
        assert!(
            matches.iter().all(|row| if expected.is_empty() {
                row[5].to_string().is_empty()
            } else {
                row[5] == expected
            }),
            "{key}"
        );
    }
    let mut sales = open_workbook_auto(cleaned.join("销售用券情况统计.xlsx")).unwrap();
    let sales = sales.worksheet_range_at(0).unwrap().unwrap();
    for key in only_exchanges {
        let matches: Vec<_> = sales.rows().skip(1).filter(|row| row[9] == key).collect();
        assert!(!matches.is_empty(), "{key}");
        assert!(
            matches
                .iter()
                .all(|row| !row[10].to_string().starts_with("退货")),
            "{key}"
        );
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
