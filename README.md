# subsidy

Rust 国补数据处理工具：读取原始 Excel，完成8类清洗并生成上传汇总、门店财务统筹、家电台账与数码台账4份报表。

## 开发

安装 Rustup 后，在项目根目录运行；工具链和依赖分别以 `rust-toolchain.toml`、`Cargo.toml` 与 `Cargo.lock` 为准。

```bash
cargo run --locked
cargo build --release --locked
```

按提示输入原始文件夹路径。源文件只读；明细与报表分别写入其同级 `国补明细/`、`国补报表/`。输入文件契约见[清洗规则 §1.1](docs/cleaning-rules.md#11-输入与输出路径)。

首次克隆后执行 `git config core.hooksPath .githooks` 启用提交检查。手动检查运行 `.githooks/pre-commit`，覆盖格式、Clippy、测试和文档链接，与 CI 一致。

## 开发文档

- [开发规则](AGENTS.md)：按任务选择文档与修改约束。
- [业务术语](CONTEXT.md)：销售、上传记录与统计口径。
- [项目架构](docs/architecture.md)：模块边界、规则所有者、共享与发布。
- [清洗规则](docs/cleaning-rules.md)：源字段、映射、匹配与验收。
- [报表规则](docs/reporting-rules.md)：指标、财务匹配与工作表结构。
- [真实数据核对](docs/agents/real-data.md)：改动前后输出对比与批次验收。
