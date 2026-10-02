use std::io::{self, Write};

use crate::io::paths::resolve_input_dir;
use crate::model::ProcessError;

use super::runner;

/// 程序入口：读取源文件夹路径后执行全部类别一次，处理完成即退出。
pub fn run() -> Result<bool, ProcessError> {
    let Some(input_dir) = read_input_dir()? else {
        return Ok(true);
    };

    runner::run_all(&input_dir)
}

/// 读取源文件夹路径，无效时重新提示；读到输入结束（EOF）时返回`None`。
fn read_input_dir() -> Result<Option<std::path::PathBuf>, ProcessError> {
    loop {
        let Some(raw) = read_line("请输入源文件夹路径：")? else {
            return Ok(None);
        };

        match resolve_input_dir(&raw) {
            Ok(dir) => return Ok(Some(dir)),
            Err(error) => println!("{error}"),
        }
    }
}

/// 读取一行输入，去除首尾空白；`read_line()`返回 0 字节（EOF）时返回`None`。
fn read_line(prompt: &str) -> Result<Option<String>, ProcessError> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut buffer = String::new();
    let bytes_read = io::stdin().read_line(&mut buffer)?;
    if bytes_read == 0 {
        return Ok(None);
    }
    Ok(Some(buffer.trim().to_string()))
}
