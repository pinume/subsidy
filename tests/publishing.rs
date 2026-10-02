//! `io::publisher` 的发布、覆盖与失败恢复行为。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use subsidy::io::publisher;
use subsidy::model::ProcessError;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "clean-publishing-test-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn publishes_new_file_when_target_absent() {
    let dir = temp_dir("new");
    let temp = publisher::temp_path(&dir, "样本");
    std::fs::write(&temp, b"data").unwrap();

    let target = publisher::publish(&dir, "样本.xlsx", &temp).unwrap();

    assert_eq!(target, dir.join("样本.xlsx"));
    assert_eq!(std::fs::read(&target).unwrap(), b"data");
    assert!(!temp.exists(), "临时文件应在发布后被清理");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn overwrites_existing_target_and_removes_backup() {
    let dir = temp_dir("overwrite");
    let target_path = dir.join("样本.xlsx");
    std::fs::write(&target_path, b"old").unwrap();

    let temp = publisher::temp_path(&dir, "样本");
    std::fs::write(&temp, b"new").unwrap();

    let target = publisher::publish(&dir, "样本.xlsx", &temp).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    assert!(
        !dir.join(".样本.xlsx.bak").exists(),
        "全部成功后应删除 .bak 备份"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn empty_temp_file_is_rejected_and_original_result_kept() {
    // 模拟“写入中途失败”：临时文件已创建但内容为空。
    let dir = temp_dir("empty-temp");
    let target_path = dir.join("样本.xlsx");
    std::fs::write(&target_path, b"old").unwrap();

    let temp = publisher::temp_path(&dir, "样本");
    std::fs::write(&temp, b"").unwrap();

    let error = publisher::publish(&dir, "样本.xlsx", &temp).unwrap_err();

    assert!(matches!(error, ProcessError::Io(_)));
    assert_eq!(
        std::fs::read(&target_path).unwrap(),
        b"old",
        "发布失败时应保留原有正式结果"
    );
    assert!(!temp.exists(), "失败后应清理本次临时文件");
    assert!(!dir.join(".样本.xlsx.bak").exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_temp_file_is_reported_and_original_result_kept() {
    let dir = temp_dir("missing-temp");
    let target_path = dir.join("样本.xlsx");
    std::fs::write(&target_path, b"old").unwrap();

    let missing_temp = dir.join(".样本.missing.xlsx.tmp");

    let error = publisher::publish(&dir, "样本.xlsx", &missing_temp).unwrap_err();

    assert!(matches!(error, ProcessError::Io(_)));
    assert_eq!(std::fs::read(&target_path).unwrap(), b"old");

    std::fs::remove_dir_all(&dir).unwrap();
}
