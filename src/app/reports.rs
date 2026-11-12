use std::cell::OnceCell;
use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use crate::io::{
    paths::{REPORT_DIR, validate_output_dir},
    publisher,
};
use crate::jobs::{self, Category};
use crate::model::ProcessError;
use crate::reporting::{data::CommonInputs, finance_wb, ledger, summary_wb};

const SUMMARY_DEPENDENCIES: &[Category] = &[
    Category::Invoice,
    Category::Coupons,
    Category::UploadedAppliance,
    Category::UploadedDigital,
    Category::RefundAppliance,
    Category::RefundDigital,
];

const FINANCE_DEPENDENCIES: &[Category] = &[
    Category::Invoice,
    Category::Coupons,
    Category::UploadedAppliance,
    Category::UploadedDigital,
    Category::RefundAppliance,
    Category::RefundDigital,
    Category::UnionPay,
];

pub(super) fn run_all(input: &Path, cleaned: &Path, success: &HashSet<Category>) -> bool {
    let output = input
        .parent()
        .expect("validated input has parent")
        .join(REPORT_DIR);
    let common_inputs = OnceCell::new();
    let mut all_ok = true;
    for (stem, dependencies, generator) in [
        (
            "国补上传情况汇总",
            SUMMARY_DEPENDENCIES,
            summary_wb::generate_with_inputs
                as fn(&Path, &Path, &CommonInputs) -> Result<Vec<String>, String>,
        ),
        (
            "26年国补门店财务统筹表",
            FINANCE_DEPENDENCIES,
            finance_wb::generate_with_inputs,
        ),
    ] {
        let missing: Vec<_> = jobs::registry()
            .iter()
            .filter(|job| {
                dependencies.contains(&job.category()) && !success.contains(&job.category())
            })
            .map(|job| format!("{}.xlsx", job.output_stem()))
            .collect();
        if !missing.is_empty() {
            println!(
                "[{stem}] 跳过报表：本次未成功发布 {}，不使用旧文件。",
                missing.join("、")
            );
            all_ok = false;
            continue;
        }
        all_ok &= attempt(stem, || {
            validate_output_dir(input)?;
            generate_and_publish(stem, cleaned, &output, |input, path| {
                let inputs = common_inputs
                    .get_or_init(|| CommonInputs::load(input))
                    .as_ref()
                    .map_err(Clone::clone)?;
                generator(input, path, inputs)
            })
        });
    }
    // 台账直接读取原始数据，不依赖本次清洗结果。
    for (stem, generator) in [
        (
            "家电台账",
            ledger::generate_appliance as fn(&Path, &Path) -> Result<Vec<String>, String>,
        ),
        ("数码台账", ledger::generate_digital),
    ] {
        all_ok &= attempt(stem, || {
            validate_output_dir(input)?;
            generate_and_publish(stem, input, &output, generator)
        });
    }
    all_ok
}

/// 隔离单份报表的失败与 panic，打印结果；成功（含告警）时返回 `true`。
fn attempt(stem: &str, run: impl FnOnce() -> Result<Vec<String>, ProcessError>) -> bool {
    let result = panic::catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|payload| {
        Err(ProcessError::Read(format!(
            "报表处理过程中发生程序内部错误：{}",
            super::runner::panic_message(payload.as_ref())
        )))
    });
    match result {
        Ok(warnings) => {
            for warning in warnings {
                println!("[{stem}] 注意：{warning}");
            }
            println!("[{stem}] 报表成功。");
            true
        }
        Err(error) => {
            println!("[{stem}] 报表失败：{error}");
            false
        }
    }
}

fn generate_and_publish(
    stem: &str,
    input: &Path,
    output: &Path,
    generator: impl FnOnce(&Path, &Path) -> Result<Vec<String>, String>,
) -> Result<Vec<String>, ProcessError> {
    std::fs::create_dir_all(output)?;
    let temporary = publisher::temp_path(output, stem);
    let mut guard = publisher::TempFile::new(temporary.clone());
    let warnings = match generator(input, &temporary).map_err(ProcessError::Read) {
        Ok(warnings) => warnings,
        Err(error) => return Err(guard.fail(error)),
    };
    guard.disarm(); // publisher takes ownership of cleanup and rollback.
    let path = publisher::publish(output, &format!("{stem}.xlsx"), &temporary)?;
    println!("已发布：{}", path.display());
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_path;

    #[test]
    fn both_report_attempts_parse_each_input_once_even_when_summary_fails() {
        let root = unique_temp_path("report-input-count");
        let input = root.join("raw");
        let cleaned = root.join("国补明细");
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir(&cleaned).unwrap();
        let names = [
            "销售用券情况统计.xlsx",
            "已上传家电电脑.xlsx",
            "已上传数码.xlsx",
            "发票明细.xlsx",
            "回款明细家电电脑.xlsx",
            "回款明细数码.xlsx",
            "银联交易明细门店.xlsx",
        ];
        for name in names {
            let mut book = rust_xlsxwriter::Workbook::new();
            let sheet = book.add_worksheet();
            sheet.write_string(0, 0, "商户号").unwrap();
            sheet.write_string(0, 1, "状态").unwrap();
            sheet.write_string(1, 0, "001").unwrap();
            sheet.write_string(1, 1, "审核终止").unwrap();
            book.save(cleaned.join(name)).unwrap();
        }
        let success = jobs::registry().iter().map(|job| job.category()).collect();
        crate::reporting::reader::take_read_paths();
        // Tables parse, but missing business fields make both renderers fail independently.
        assert!(!run_all(&input, &cleaned, &success));
        let paths = crate::reporting::reader::take_read_paths();
        assert_eq!(paths.len(), 7);
        for name in names {
            assert_eq!(
                paths
                    .iter()
                    .filter(|path| **path == cleaned.join(name))
                    .count(),
                1
            );
        }
        assert_eq!(std::fs::read_dir(root.join(REPORT_DIR)).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ledgers_run_without_cleaning_and_a_failed_ledger_keeps_its_old_file() {
        let root = unique_temp_path("report-ledgers");
        let input = root.join("raw");
        let reports = root.join(REPORT_DIR);
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(reports.join("家电台账.xlsx"), b"old ledger").unwrap();
        let mut book = rust_xlsxwriter::Workbook::new();
        book.add_worksheet().write_string(0, 0, "商户编号").unwrap();
        book.save(input.join("2026年数码补贴明细已经回款下发.xlsx"))
            .unwrap();
        let mut invoices = rust_xlsxwriter::Workbook::new();
        let sheet = invoices.add_worksheet();
        for (col, header) in crate::jobs::invoice::SOURCE_HEADERS.iter().enumerate() {
            sheet.write_string(5, col as u16, *header).unwrap();
        }
        invoices.save(input.join("发票_20260201.xlsx")).unwrap();
        // 清洗全部失败：原有报表跳过；家电台账缺少补贴明细失败；数码台账照常发布。
        assert!(!run_all(&input, &root.join("国补明细"), &HashSet::new()));
        assert_eq!(
            std::fs::read(reports.join("家电台账.xlsx")).unwrap(),
            b"old ledger"
        );
        assert!(reports.join("数码台账.xlsx").is_file());
        assert_eq!(std::fs::read_dir(&reports).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn report_failures_preserve_old_files_clean_temps_and_allow_next_report() {
        for stage in ["generation", "missing-output", "panic", "publication"] {
            let output = unique_temp_path("report-failure");
            std::fs::create_dir(&output).unwrap();
            let xlsx = output.join("summary.xlsx");
            std::fs::write(&xlsx, b"old xlsx").unwrap();
            if stage == "publication" {
                publisher::inject(&[("replace", false)]);
            }
            let result = panic::catch_unwind(|| {
                generate_and_publish("summary", &output, &output, |_, path| {
                    if stage != "missing-output" {
                        std::fs::write(path, b"new xlsx").unwrap();
                    }
                    match stage {
                        "generation" => Err("injected generation failure".into()),
                        "panic" => panic!("injected generation panic"),
                        _ => Ok(Vec::new()),
                    }
                })
            });
            publisher::inject(&[]);
            if stage == "panic" {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert_eq!(std::fs::read(&xlsx).unwrap(), b"old xlsx");
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
            generate_and_publish("finance", &output, &output, |_, path| {
                std::fs::write(path, b"finance").map_err(|error| error.to_string())?;
                Ok(Vec::new())
            })
            .unwrap();
            assert_eq!(
                std::fs::read(output.join("finance.xlsx")).unwrap(),
                b"finance"
            );
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
            std::fs::remove_dir_all(output).unwrap();
        }
    }

    #[test]
    fn report_generation_surfaces_warnings_and_publishes_successfully() {
        let output = unique_temp_path("report-warning-propagation");
        std::fs::create_dir(&output).unwrap();
        let expected_warnings = vec![
            "发票商品名称索引：1 个键对应多条内容不同的记录，已按未命中处理（INV001）".to_string(),
        ];
        let warnings = generate_and_publish("test-summary", &output, &output, |_, path| {
            std::fs::write(path, b"new report").map_err(|error| error.to_string())?;
            Ok(expected_warnings.clone())
        })
        .unwrap();
        assert_eq!(warnings, expected_warnings);
        assert_eq!(
            std::fs::read(output.join("test-summary.xlsx")).unwrap(),
            b"new report"
        );
        std::fs::remove_dir_all(output).unwrap();
    }
}
