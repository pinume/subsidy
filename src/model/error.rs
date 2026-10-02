use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("路径无效：{path}（{reason}）", path = path.display())]
    InvalidPath { path: PathBuf, reason: String },

    #[error("未找到符合规则的源文件：{pattern}")]
    NoInput { pattern: String },

    #[error("{file} / {sheet}：结构异常：{detail}")]
    Structure {
        file: String,
        sheet: String,
        detail: String,
    },

    #[error("{file} / {sheet} 第{row}行 [{field}]：数据异常，原值“{value}”：{detail}")]
    Data {
        file: String,
        sheet: String,
        row: u32,
        field: String,
        value: String,
        detail: String,
    },

    #[error("疑似重复导出：{detail}")]
    Duplicate { detail: String },

    #[error("读取失败：{0}")]
    Read(String),

    #[error("写入或发布失败：{0}")]
    Io(#[from] std::io::Error),
}
