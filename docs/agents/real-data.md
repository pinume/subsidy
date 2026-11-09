# 真实数据核对

改动可能影响清洗或报表输出时，用真实数据对比改动前后的结果。

## 数据位置

- 最新批次：`~/yz/data`。该目录会被整批替换，先看文件日期确认是哪一批。

## 步骤

1. 复制数据到临时目录（输出写在源目录的上一级，`国补明细/` 与 `国补报表/`），原始文件只读：
   `cp -r ~/yz/data <run>/data && (cd <run> && md5sum data/* > src.md5)`
2. 分别构建改动前后的 release 二进制（改动前可用 `git worktree add <dir> HEAD`），各自对一份副本运行：
   `echo "<run>/data" | target/release/subsidy`
3. 比较两次输出：`scripts/compare-outputs.sh <旧run> <新run>`；有不一致的文件时，用 `uv run -q --with openpyxl python -I scripts/diff-cells.py <旧run> <新run>` 看具体单元格，核对是否都来自本次改动的规则。
4. 确认源文件未变：`(cd <run> && md5sum -c --quiet src.md5)`。

完成标准：两次输出全部一致，或每一处差异都能对应到本次改动的规则。

## 验收测试

`COMBINE_SOURCE_DIR=~/yz/data cargo test --release --locked --test acceptance_test -- --ignored`

数据会持续更新，测试只断言任意批次都成立的关系（明细与报表行数对应、汇总各列相加、退货链独立验算、失败隔离），不写死批次计数；新增断言也遵循这一点。

## 环境限制

- `perf` 不可用（`perf_event_paranoid=4`）；分阶段耗时可给 CLI 每行输出加时间戳估算。
- 系统 Python 没有 `openpyxl`：`uv run -q --with openpyxl python -I <脚本>`。
