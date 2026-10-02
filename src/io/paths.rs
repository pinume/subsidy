use std::path::{Path, PathBuf};

use crate::model::ProcessError;

pub(crate) const DETAIL_DIR: &str = "国补明细";
pub(crate) const REPORT_DIR: &str = "国补报表";

/// 去除首尾空白，并在结果首尾为成对单引号或双引号时去除该对引号。
fn normalize_raw_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    unquoted.trim().to_string()
}

/// 解析用户输入的源文件夹路径：去首尾空白与成对引号，相对路径以当前工作目录为基准
/// 解析为绝对路径，并确认该路径存在、是文件夹且可读取。
pub fn resolve_input_dir(raw: &str) -> Result<PathBuf, ProcessError> {
    let normalized = normalize_raw_path(raw);
    if normalized.is_empty() {
        return Err(ProcessError::InvalidPath {
            path: PathBuf::new(),
            reason: "路径为空".to_string(),
        });
    }

    let candidate = Path::new(&normalized);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        std::env::current_dir()?.join(candidate)
    };

    if !absolute.exists() {
        return Err(ProcessError::InvalidPath {
            path: absolute,
            reason: "路径不存在".to_string(),
        });
    }
    if !absolute.is_dir() {
        return Err(ProcessError::InvalidPath {
            path: absolute,
            reason: "不是文件夹".to_string(),
        });
    }
    std::fs::read_dir(&absolute).map_err(|source| ProcessError::InvalidPath {
        path: absolute.clone(),
        reason: format!("不可读取：{source}"),
    })?;

    validate_output_dir(&absolute)?;
    Ok(absolute)
}

/// 列出`输入目录`直接包含的 `.xlsx` 文件，不递归，排除 `~$` 开头的临时文件。
pub fn list_xlsx_files(dir: &Path) -> Result<Vec<PathBuf>, ProcessError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        #[cfg(test)]
        super::publisher::fault("directory-entry")?;
        let path = entry?.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| !name.starts_with("~$"))
            && path.extension().is_some_and(|ext| ext == "xlsx")
        {
            files.push(path);
        }
    }
    Ok(files)
}

/// 在创建输出目录前比较真实路径，避免覆盖源数据（包括符号链接）。
pub fn validate_output_dir(input_dir: &Path) -> Result<(), ProcessError> {
    let input = std::fs::canonicalize(input_dir)?;
    let parent = input_dir
        .parent()
        .ok_or_else(|| ProcessError::InvalidPath {
            path: input_dir.to_path_buf(),
            reason: "无法确定父目录".to_string(),
        })?;
    let cleaned = parent.join(DETAIL_DIR);
    let reports = parent.join(REPORT_DIR);
    let real_cleaned = resolve_output_dir(&cleaned)?;
    let real_reports = resolve_output_dir(&reports)?;
    for (output, real_output) in [(&cleaned, &real_cleaned), (&reports, &real_reports)] {
        if real_output.starts_with(&input) || input.starts_with(real_output) {
            return Err(ProcessError::InvalidPath {
                path: output.clone(),
                reason: "输出目录与源目录重叠，拒绝处理以保护源数据".to_string(),
            });
        }
    }
    if real_cleaned.starts_with(&real_reports) || real_reports.starts_with(&real_cleaned) {
        return Err(ProcessError::InvalidPath {
            path: reports,
            reason: "两个输出目录重叠，拒绝处理以保护源数据".to_string(),
        });
    }
    Ok(())
}

fn resolve_output_dir(output: &Path) -> Result<PathBuf, ProcessError> {
    let real_output = match std::fs::canonicalize(output) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // 已存在但失效的符号链接不能按普通待创建目录处理。
            if std::fs::symlink_metadata(output).is_ok() {
                return Err(error.into());
            }
            std::fs::canonicalize(output.parent().expect("output has parent"))?
                .join(output.file_name().expect("output has filename"))
        }
        Err(error) => return Err(error.into()),
    };
    if real_output.exists() && !real_output.is_dir() {
        return Err(ProcessError::InvalidPath {
            path: output.to_path_buf(),
            reason: "输出路径不是文件夹".to_string(),
        });
    }
    Ok(real_output)
}

/// 计算并确保`输入目录`的父目录下的`输出目录`（`国补明细/`）存在。
pub fn ensure_output_dir(input_dir: &Path) -> Result<PathBuf, ProcessError> {
    let parent = input_dir
        .parent()
        .ok_or_else(|| ProcessError::InvalidPath {
            path: input_dir.to_path_buf(),
            reason: "无法确定父目录".to_string(),
        })?;
    let output_dir = parent.join(DETAIL_DIR);
    validate_output_dir(input_dir)?;
    std::fs::create_dir_all(&output_dir)?;
    Ok(output_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::unique_temp_path;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = unique_temp_path(&format!("paths-test-{label}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_raw_path("  /a/b  "), "/a/b");
    }

    #[test]
    fn normalize_strips_matching_single_quotes() {
        assert_eq!(normalize_raw_path("'/a/b'"), "/a/b");
    }

    #[test]
    fn normalize_strips_matching_double_quotes() {
        assert_eq!(normalize_raw_path("  \"/a/b\"  "), "/a/b");
    }

    #[test]
    fn normalize_keeps_unmatched_quote() {
        assert_eq!(normalize_raw_path("'/a/b"), "'/a/b");
    }

    #[test]
    fn resolve_input_dir_rejects_empty_input() {
        let error = resolve_input_dir("   ").unwrap_err();
        assert!(matches!(error, ProcessError::InvalidPath { .. }));
    }

    #[test]
    fn resolve_input_dir_rejects_missing_path() {
        let dir = scratch_dir("missing-base");
        let missing = dir.join("does-not-exist");
        let error = resolve_input_dir(missing.to_str().unwrap()).unwrap_err();
        assert!(matches!(error, ProcessError::InvalidPath { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_input_dir_rejects_file_path() {
        let dir = scratch_dir("file-not-dir");
        let file = dir.join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        let error = resolve_input_dir(file.to_str().unwrap()).unwrap_err();
        assert!(matches!(error, ProcessError::InvalidPath { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_input_dir_accepts_quoted_absolute_dir() {
        let dir = scratch_dir("ok-dir");
        let quoted = format!("  \"{}\"  ", dir.display());
        let resolved = resolve_input_dir(&quoted).unwrap();
        assert_eq!(resolved, dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_xlsx_files_filters_by_extension_and_temp_prefix() {
        let dir = scratch_dir("list-xlsx");
        std::fs::write(dir.join("发票_20260914.xlsx"), b"x").unwrap();
        std::fs::write(dir.join("~$发票_20260914.xlsx"), b"x").unwrap();
        std::fs::write(dir.join("说明.txt"), b"x").unwrap();
        std::fs::create_dir(dir.join("子目录.xlsx")).unwrap();

        let mut files: Vec<String> = list_xlsx_files(&dir)
            .unwrap()
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        files.sort();

        assert_eq!(files, vec!["发票_20260914.xlsx".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_output_dir_creates_sibling_output_folder() {
        let base = scratch_dir("output-base");
        let input_dir = base.join("源数据");
        std::fs::create_dir_all(&input_dir).unwrap();

        let output_dir = ensure_output_dir(&input_dir).unwrap();

        assert_eq!(output_dir, base.join("国补明细"));
        assert!(output_dir.is_dir());
        std::fs::remove_dir_all(&base).unwrap();
    }
    #[test]
    fn rejects_source_output_overlap_before_creation() {
        let base = scratch_dir("overlap");
        let input = base.join("国补明细");
        std::fs::create_dir(&input).unwrap();
        std::fs::write(input.join("source.xlsx"), b"source").unwrap();
        assert!(
            resolve_input_dir(input.to_str().unwrap())
                .unwrap_err()
                .to_string()
                .contains("保护源数据")
        );
        assert!(ensure_output_dir(&input).is_err());
        assert_eq!(std::fs::read(input.join("source.xlsx")).unwrap(), b"source");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_output_symlink_into_source_and_source_alias() {
        let base = scratch_dir("symlink");
        let input = base.join("input");
        std::fs::create_dir(&input).unwrap();
        std::os::unix::fs::symlink(input.join("nested"), base.join("国补明细")).unwrap();
        assert!(ensure_output_dir(&input).is_err());
        assert!(!input.join("nested").exists());
        std::fs::create_dir(input.join("nested")).unwrap();
        assert!(resolve_input_dir(input.to_str().unwrap()).is_err());
        std::fs::remove_file(base.join("国补明细")).unwrap();
        std::os::unix::fs::symlink(&input, base.join("国补明细")).unwrap();
        assert!(resolve_input_dir(input.to_str().unwrap()).is_err());
        assert!(resolve_input_dir(base.join("国补明细").to_str().unwrap()).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn directory_entry_error_is_propagated() {
        let dir = scratch_dir("directory-error");
        std::fs::write(dir.join("source.xlsx"), b"source").unwrap();
        super::super::publisher::inject(&[("directory-entry", false)]);
        assert!(
            list_xlsx_files(&dir)
                .unwrap_err()
                .to_string()
                .contains("directory-entry")
        );
        assert_eq!(list_xlsx_files(&dir).unwrap().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
