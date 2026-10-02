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
    for (stem, dependencies, generator, markdown) in [
        (
            "国补上传情况汇总",
            SUMMARY_DEPENDENCIES,
            summary_wb::generate_with_inputs
                as fn(&Path, &Path, &CommonInputs) -> Result<(), String>,
            true,
        ),
        (
            "26年国补门店财务统筹表",
            FINANCE_DEPENDENCIES,
            finance_wb::generate_with_inputs,
            false,
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
            generate_and_publish(
                stem,
                cleaned,
                &output,
                |input, path| {
                    let inputs = common_inputs
                        .get_or_init(|| CommonInputs::load(input))
                        .as_ref()
                        .map_err(Clone::clone)?;
                    generator(input, path, inputs)
                },
                markdown,
            )
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
    markdown: bool,
) -> Result<(), ProcessError> {
    std::fs::create_dir_all(output)?;
    let temporary = publisher::temp_path(output, stem);
    let temporary_md = temporary.with_extension("md");
    let mut guards = vec![publisher::TempFile::new(temporary.clone())];
    let mut files = vec![(temporary, format!("{stem}.xlsx"))];
    if markdown {
        guards.push(publisher::TempFile::new(temporary_md.clone()));
        files.push((temporary_md, format!("{stem}.md")));
    }
    let generated = (|| {
        generator(input, &files[0].0).map_err(ProcessError::Read)?;
        for (temp, _) in &files {
            let metadata = std::fs::metadata(temp)?;
            if !metadata.is_file() || metadata.len() == 0 {
                return Err(
                    std::io::Error::other(format!("临时结果为空：{}", temp.display())).into(),
                );
            }
        }
        Ok(())
    })();
    if let Err(mut error) = generated {
        for guard in &mut guards {
            error = guard.fail(error);
        }
        return Err(error);
    }

    let mut published = Vec::new();
    for ((temp, filename), guard) in files.iter().zip(&mut guards) {
        guard.disarm(); // publisher takes ownership of cleanup and rollback.
        match publisher::publish(output, filename, temp) {
            Ok(path) => {
                println!("已发布：{}", path.display());
                published.push(filename.as_str());
            }
            Err(error) => {
                let mut error = ProcessError::Read(format!(
                    "发布 {filename} 失败：{error}；本次已发布：{}",
                    if published.is_empty() {
                        "无".to_string()
                    } else {
                        published.join("、")
                    }
                ));
                for guard in &mut guards {
                    error = guard.fail(error);
                }
                return Err(error);
            }
        }
    }
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
        for stage in [
            "generation",
            "missing-markdown",
            "panic",
            "publish-markdown",
        ] {
            let output = unique_temp_path("report-failure");
            std::fs::create_dir(&output).unwrap();
            let xlsx = output.join("summary.xlsx");
            let md = output.join("summary.md");
            std::fs::write(&xlsx, b"old xlsx").unwrap();
            if stage == "publish-markdown" {
                std::fs::create_dir(&md).unwrap();
                std::fs::write(md.join("keep"), b"old markdown").unwrap();
            } else {
                std::fs::write(&md, b"old markdown").unwrap();
            }
            let result = panic::catch_unwind(|| {
                generate_and_publish(
                    "summary",
                    &output,
                    &output,
                    |_, path| {
                        std::fs::write(path, b"new xlsx").unwrap();
                        if stage != "missing-markdown" {
                            std::fs::write(path.with_extension("md"), b"new markdown").unwrap();
                        }
                        match stage {
                            "generation" => Err("injected generation failure".into()),
                            "panic" => panic!("injected generation panic"),
                            _ => Ok(()),
                        }
                    },
                    true,
                )
            });
            if stage == "panic" {
                assert!(result.is_err());
            } else {
                let error = result.unwrap().unwrap_err().to_string();
                if stage == "publish-markdown" {
                    assert!(error.contains("本次已发布：summary.xlsx"), "{error}");
                }
            }
            assert_eq!(
                std::fs::read(&xlsx).unwrap(),
                if stage == "publish-markdown" {
                    b"new xlsx"
                } else {
                    b"old xlsx"
                }
            );
            assert_eq!(
                std::fs::read(if md.is_dir() { md.join("keep") } else { md }).unwrap(),
                b"old markdown"
            );
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
            generate_and_publish(
                "finance",
                &output,
                &output,
                |_, path| std::fs::write(path, b"finance").map_err(|error| error.to_string()),
                false,
            )
            .unwrap();
            assert_eq!(
                std::fs::read(output.join("finance.xlsx")).unwrap(),
                b"finance"
            );
            assert_eq!(std::fs::read_dir(&output).unwrap().count(), 3);
            std::fs::remove_dir_all(output).unwrap();
        }
    }
}
