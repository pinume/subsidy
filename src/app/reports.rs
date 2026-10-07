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
use crate::reporting::{data::CommonInputs, finance_wb, summary_wb};

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
                as fn(&Path, &Path, &CommonInputs) -> Result<(), String>,
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
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            validate_output_dir(input)?;
            generate_and_publish(stem, cleaned, &output, |input, path| {
                let inputs = common_inputs
                    .get_or_init(|| CommonInputs::load(input))
                    .as_ref()
                    .map_err(Clone::clone)?;
                generator(input, path, inputs)
            })
        }))
        .unwrap_or_else(|payload| {
            Err(ProcessError::Read(format!(
                "报表处理过程中发生程序内部错误：{}",
                super::runner::panic_message(payload.as_ref())
            )))
        });
        match result {
            Ok(()) => println!("[{stem}] 报表成功。"),
            Err(error) => {
                println!("[{stem}] 报表失败：{error}");
                all_ok = false;
            }
        }
    }
    all_ok
}

fn generate_and_publish(
    stem: &str,
    input: &Path,
    output: &Path,
    generator: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), ProcessError> {
    std::fs::create_dir_all(output)?;
    let temporary = publisher::temp_path(output, stem);
    let mut guard = publisher::TempFile::new(temporary.clone());
    if let Err(error) = generator(input, &temporary).map_err(ProcessError::Read) {
        return Err(guard.fail(error));
    }
    guard.disarm(); // publisher takes ownership of cleanup and rollback.
    let path = publisher::publish(output, &format!("{stem}.xlsx"), &temporary)?;
    println!("已发布：{}", path.display());
    Ok(())
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
                        _ => Ok(()),
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
                std::fs::write(path, b"finance").map_err(|error| error.to_string())
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
}
