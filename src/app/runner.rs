use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use crate::io::paths::ensure_output_dir;
use crate::io::publisher;
use crate::io::xlsx_writer::write_table;
use crate::jobs::{self, Category, Job};
use crate::model::{ProcessError, Table};

/// 按注册顺序依次执行全部八类数据；单个类别失败或 panic 不中断其余类别，
/// 逐项报告各自的成功或失败结果。
pub fn run_all(input_dir: &Path) -> Result<bool, ProcessError> {
    let cleaned = ensure_output_dir(input_dir)?;
    let _lock = acquire_run_lock(&cleaned)?;
    let success = clean_all(input_dir);
    let reports_ok = super::reports::run_all(input_dir, &cleaned, &success);
    Ok(success.len() == jobs::registry().len() && reports_ok)
}

fn clean_all(input_dir: &Path) -> HashSet<Category> {
    let mut ctx = jobs::PipelineContext::new();
    let registry = jobs::registry();
    let mut success = HashSet::new();
    let mut failure_count = 0;

    for job in registry {
        if execute(*job, input_dir, &mut ctx) {
            success.insert(job.category());
        } else {
            failure_count += 1;
        }
    }

    println!("--------------------------------------------------");
    println!(
        "全部类别执行完毕：共 {} 项，成功 {}，失败 {}。",
        registry.len(),
        success.len(),
        failure_count
    );
    success
}

fn execute(job: &dyn Job, input_dir: &Path, ctx: &mut jobs::PipelineContext) -> bool {
    let title = job.title();
    let outcome = process(job, input_dir, ctx);

    match outcome {
        Ok(path) => {
            println!("[{title}] 处理成功：{}", path.display());
            if job.category() == jobs::Category::Coupons {
                ctx.print_stats();
            }
            true
        }
        Err(error) => {
            println!("[{title}] 处理失败：{error}");
            false
        }
    }
}

fn process(
    job: &dyn Job,
    input_dir: &Path,
    ctx: &mut jobs::PipelineContext,
) -> Result<PathBuf, ProcessError> {
    // 用 catch_unwind 隔离单个任务的 panic，避免其中断批量执行或整个程序。
    panic::catch_unwind(AssertUnwindSafe(|| {
        let table = job.run_in_context(input_dir, ctx)?;
        publish(job, input_dir, &table)
    }))
    .unwrap_or_else(|payload| {
        Err(ProcessError::Read(format!(
            "处理过程中发生程序内部错误：{}",
            panic_message(payload.as_ref())
        )))
    })
}

fn publish(job: &dyn Job, input_dir: &Path, table: &Table) -> Result<PathBuf, ProcessError> {
    let output_dir = ensure_output_dir(input_dir)?;
    let stem = job.output_stem();
    let temp = publisher::temp_path(&output_dir, stem);
    let mut guard = publisher::TempFile::new(temp.clone());
    if let Err(error) = write_table(table, &temp) {
        return Err(guard.fail(error));
    }
    guard.disarm(); // publish 接管临时文件及回滚守卫。
    publisher::publish(&output_dir, &format!("{stem}.xlsx"), &temp)
}

pub(super) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("未知错误")
        .to_string()
}

fn acquire_run_lock(output: &Path) -> Result<std::fs::File, ProcessError> {
    let path = output.join(".combine.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    file.try_lock().map_err(|error| {
        std::io::Error::other(format!(
            "无法取得运行锁 {}：{error}；已有任务运行或文件系统不支持文件锁",
            path.display()
        ))
    })?;
    Ok(file) // The OS releases the lock when this handle closes, including process termination.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column, ColumnType, Row, Value};
    use crate::test_support::unique_temp_path;

    struct SampleJob(bool);
    impl Job for SampleJob {
        fn category(&self) -> jobs::Category {
            jobs::Category::Invoice
        }
        fn title(&self) -> &'static str {
            "sample"
        }
        fn output_stem(&self) -> &'static str {
            if self.0 { "first" } else { "second" }
        }
        fn run_in_context(
            &self,
            _: &Path,
            _: &mut jobs::PipelineContext,
        ) -> Result<Table, ProcessError> {
            #[cfg(test)]
            publisher::fault("business")?;
            Ok(Table {
                columns: vec![Column {
                    name: "value",
                    ty: ColumnType::Text,
                }],
                rows: vec![Row {
                    values: vec![Value::Text("new".into())],
                    fill: None,
                }],
            })
        }
    }

    #[test]
    fn all_lifecycle_failures_are_isolated_and_next_category_publishes() {
        for stage in ["business", "write", "replace", "cleanup-backup"] {
            for panic in [false, true] {
                let base = unique_temp_path("runner-faults");
                let input = base.join("input");
                std::fs::create_dir_all(&input).unwrap();
                let output = ensure_output_dir(&input).unwrap();
                let old = output.join("first.xlsx");
                std::fs::write(&old, b"old").unwrap();
                publisher::inject(&[(stage, panic)]);
                let mut ctx = jobs::PipelineContext::new();
                let error = process(&SampleJob(true), &input, &mut ctx).unwrap_err();
                assert!(error.to_string().contains(stage), "{error}");
                assert_eq!(std::fs::read(&old).unwrap(), b"old");
                assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
                assert!(
                    process(&SampleJob(false), &input, &mut ctx)
                        .unwrap()
                        .is_file()
                );
                assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
                std::fs::remove_dir_all(base).unwrap();
            }
        }
    }
}
