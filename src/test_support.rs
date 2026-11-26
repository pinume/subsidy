//! 仅测试使用的公共小工具：`src/`内的单元测试按模块引用；集成测试（`tests/`）是独立
//! 编译单元，通过`#[path]`引入本文件，不经生产库的公开接口。

/// 生成一个进程内唯一的临时路径：`<系统临时目录>/<label>-<pid>-<纳秒时间戳>`。
#[cfg(test)]
pub(crate) fn unique_temp_path(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("clean-{label}-{}-{nanos}", std::process::id()))
}
