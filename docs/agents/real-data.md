# 真实数据核对

改动可能影响清洗或报表输出时，用真实数据对比改动前后的结果。

## 数据位置

- 最新批次：`~/yz/data`。该目录会被整批替换，先看文件日期确认是哪一批。
- 2026-10-01 验收批次：不在本机常驻；需要时向用户索取路径，用于 `tests/acceptance_test.rs`。

## 步骤

1. 复制数据到临时目录（输出写在源目录的上一级，`国补明细/` 与 `国补报表/`），原始文件只读：
   `cp -r ~/yz/data <run>/data && (cd <run> && md5sum data/* > src.md5)`
2. 分别构建改动前后的 release 二进制（改动前可用 `git worktree add <dir> HEAD`），各自对一份副本运行：
   `echo "<run>/data" | target/release/subsidy`
3. 比较两次输出：`scripts/compare-outputs.sh <旧run> <新run>`。业务规则变更导致的差异，再按单元格核对是否符合预期。
4. 确认源文件未变：`(cd <run> && md5sum -c --quiet src.md5)`。

完成标准：两次输出全部一致，或每一处差异都能对应到本次改动的规则。

## 2026-10-01 验收

`COMBINE_SOURCE_DIR=<10-01原始数据> cargo test --release --locked --test acceptance_test -- --ignored --nocapture`

该批次的计数基准只写在这个测试里，不适用于其他批次。

## 环境限制

- `perf` 不可用（`perf_event_paranoid=4`）；分阶段耗时可给 CLI 每行输出加时间戳估算。
- 系统 Python 没有 `openpyxl`：`uv run -q --with openpyxl python -I <脚本>`。
