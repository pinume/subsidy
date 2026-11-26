use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// 启动 CLI，写入原始输入后关闭 stdin（触发 EOF），捕获输出并等待退出。
pub fn run_cli_process(cwd: Option<&Path>, input: &[u8]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_subsidy"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
