use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::ProcessError;

/// 每次操作独立命名，不复用历史临时文件或备份。
pub fn temp_path(output_dir: &Path, stem: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    output_dir.join(format!(
        ".{stem}.{}.{nanos:x}.{sequence}.xlsx.tmp",
        std::process::id()
    ))
}

fn remove_owned(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn with_cleanup(error: ProcessError, errors: Vec<String>) -> ProcessError {
    if errors.is_empty() {
        error
    } else {
        ProcessError::Io(std::io::Error::other(format!(
            "{error}；后续恢复/清理失败：{}",
            errors.join("；")
        )))
    }
}

/// 覆盖写入提前返回和 panic；常规错误显式清理并合并错误信息。
pub(crate) struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
    pub(crate) fn fail(&mut self, error: ProcessError) -> ProcessError {
        let errors = operation("cleanup-temp", || remove_owned(&self.path))
            .err()
            .map(|e| format!("{}：{e}", self.path.display()))
            .into_iter()
            .collect();
        self.disarm();
        with_cleanup(error, errors)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = remove_owned(&self.path)
        {
            eprintln!("清理临时文件失败，残留 {}：{error}", self.path.display());
        }
    }
}

struct Publication {
    target: PathBuf,
    backup: PathBuf,
    backed_up: bool,
    installed: bool,
    armed: bool,
}

impl Publication {
    fn rollback(&mut self) -> Vec<String> {
        self.armed = false;
        let mut errors = Vec::new();
        if self.installed {
            if let Err(error) = operation("remove-new", || remove_owned(&self.target)) {
                errors.push(format!(
                    "删除新结果 {}：{error}；备份保留于 {}",
                    self.target.display(),
                    self.backup.display()
                ));
                return errors;
            }
            self.installed = false;
        }
        if self.backed_up {
            if let Err(error) = operation("restore", || std::fs::rename(&self.backup, &self.target))
            {
                errors.push(format!(
                    "恢复 {}：{error}；备份保留于 {}",
                    self.target.display(),
                    self.backup.display()
                ));
            } else {
                self.backed_up = false;
            }
        }
        errors
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        if self.armed {
            for error in self.rollback() {
                eprintln!("发布 panic 后恢复失败：{error}");
            }
        }
    }
}

/// 校验后备份、替换；包括备份删除失败在内的错误均回滚旧结果。
pub fn publish(output_dir: &Path, filename: &str, temp: &Path) -> Result<PathBuf, ProcessError> {
    let mut temporary = TempFile::new(temp.to_path_buf());
    let mut state = Publication {
        target: output_dir.join(filename),
        backup: temp_path(output_dir, filename).with_extension("bak"),
        backed_up: false,
        installed: false,
        armed: true,
    };
    let result = (|| -> Result<(), ProcessError> {
        let metadata = operation("validate", || std::fs::metadata(temp))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(
                std::io::Error::other(format!("临时文件不是非空文件：{}", temp.display())).into(),
            );
        }
        if state.target.try_exists()? {
            if !std::fs::symlink_metadata(&state.target)?
                .file_type()
                .is_file()
            {
                return Err(std::io::Error::other(format!(
                    "正式结果不是普通文件：{}",
                    state.target.display()
                ))
                .into());
            }
            operation("backup", || std::fs::rename(&state.target, &state.backup))?;
            state.backed_up = true;
        }
        operation("replace", || std::fs::rename(temp, &state.target))?;
        state.installed = true;
        if state.backed_up {
            operation("cleanup-backup", || std::fs::remove_file(&state.backup))?;
            state.backed_up = false;
        }
        state.armed = false;
        Ok(())
    })();
    match result {
        Ok(()) => {
            temporary.disarm();
            Ok(state.target.clone())
        }
        Err(error) => {
            let error = with_cleanup(error, state.rollback());
            Err(temporary.fail(error))
        }
    }
}

fn operation<T>(stage: &str, action: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
    #[cfg(test)]
    fault(stage)?;
    #[cfg(not(test))]
    let _ = stage;
    action()
}

#[cfg(test)]
thread_local! {
    static FAULTS: std::cell::RefCell<Vec<(&'static str, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn fault(stage: &str) -> std::io::Result<()> {
    FAULTS.with(|faults| {
        let mut faults = faults.borrow_mut();
        if let Some(index) = faults.iter().position(|(name, _)| *name == stage) {
            let (_, panic) = faults.remove(index);
            drop(faults);
            if panic {
                panic!("injected {stage} panic");
            }
            return Err(std::io::Error::other(format!("injected {stage} failure")));
        }
        Ok(())
    })
}

#[cfg(test)]
pub(crate) fn inject(faults: &[(&'static str, bool)]) {
    FAULTS.with(|f| *f.borrow_mut() = faults.to_vec());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_path;

    #[test]
    fn publishing_faults_restore_or_retain_old_data_and_report_cleanup() {
        let cases: &[&[(&str, bool)]] = &[
            &[("validate", false)],
            &[("backup", false)],
            &[("replace", false)],
            &[("cleanup-backup", false)],
            &[("replace", false), ("restore", false)],
            &[("cleanup-backup", false), ("remove-new", false)],
            &[("replace", false), ("cleanup-temp", false)],
            &[("replace", true)],
            &[("cleanup-backup", true)],
            &[("cleanup-backup", true), ("restore", false)],
        ];
        for faults in cases {
            let dir = unique_temp_path("publisher-faults");
            std::fs::create_dir(&dir).unwrap();
            let target = dir.join("sample.xlsx");
            std::fs::write(&target, b"old").unwrap();
            let historical = dir.join(".sample.xlsx.bak");
            std::fs::write(&historical, b"historical backup").unwrap();
            let temp = temp_path(&dir, "sample");
            std::fs::write(&temp, b"new").unwrap();
            inject(faults);
            let outcome = std::panic::catch_unwind(|| publish(&dir, "sample.xlsx", &temp));
            let panic = faults.iter().any(|(_, panic)| *panic);
            if panic {
                assert!(outcome.is_err());
            } else {
                let error = outcome.unwrap().unwrap_err().to_string();
                for (stage, _) in *faults {
                    assert!(error.contains(stage), "{error}");
                }
                if faults.len() > 1 {
                    assert!(error.contains(&dir.display().to_string()), "{error}");
                }
            }
            let retains_backup = faults
                .iter()
                .any(|(stage, _)| ["restore", "remove-new"].contains(stage));
            let backups: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "bak") && p != &historical)
                .collect();
            if retains_backup {
                assert_eq!(backups.len(), 1);
                assert_eq!(std::fs::read(&backups[0]).unwrap(), b"old");
            } else {
                assert!(backups.is_empty());
                assert_eq!(std::fs::read(&target).unwrap(), b"old");
            }
            assert_eq!(
                temp.exists(),
                faults.iter().any(|(stage, _)| *stage == "cleanup-temp")
            );
            assert_eq!(std::fs::read(&historical).unwrap(), b"historical backup");
            inject(&[]);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
