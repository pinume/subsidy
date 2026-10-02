//! 仅测试使用的公共小工具，供`src/`内的单元测试共享；集成测试（`tests/`）是独立
//! 编译单元，无法复用这里的内容，各自保留自己的临时路径辅助函数。

/// 生成一个进程内唯一的临时路径：`<系统临时目录>/<label>-<pid>-<纳秒时间戳>`。
#[cfg(test)]
pub(crate) fn unique_temp_path(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("clean-{label}-{}-{nanos}", std::process::id()))
}
