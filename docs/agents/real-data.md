# 真实数据核对

改动影响清洗或报表输出时，使用同一批原始数据比较改动前后的结果。先确认本地批次位置与文件日期；本地常用路径为 `~/yz/data`，也可使用其他目录。

## 输出对比

1. 构建改动前后的 release 二进制；先选择覆盖本次改动的基线提交，可用 `git worktree add --detach <基线目录> <基线提交>` 建立基线工作区。将同一批原始数据分别复制到两个新建、互不重叠的运行目录的 `data/`（`<run>`分别代表旧、新运行目录），记录源文件校验和：
   `mkdir -p <run> && cp -r <源目录> <run>/data && (cd <run> && md5sum data/* > src.md5)`。
2. 各用对应版本运行：`printf '%s\n' '<run>/data' | <二进制路径>`。结果位于各自运行目录的 `国补明细/` 与 `国补报表/`。
3. 确认两次运行均成功、各生成8份明细及2份报表，且文件名集合一致，再运行 `scripts/compare-outputs.sh <旧run> <新run>` 比较 XLSX 内容（忽略生成时间）。
4. 有差异时，用 `uv run -q --with openpyxl python -I scripts/diff-cells.py <旧run> <新run>` 查看单元格，并核对格式、底色和工作表结构差异。
5. 分别执行 `(cd <run> && md5sum -c --quiet src.md5)`，确认原始副本未变。

完成标准：输出全部一致，或每处差异都能对应到本次改动的规则。失败隔离测试允许预期的缺失结果，需逐项核对失败原因和旧文件保留情况。

## 批次验收

```bash
COMBINE_SOURCE_DIR=<源目录> cargo test --release --locked --test acceptance_test -- --ignored
```

测试复制原始数据后经完整 CLI 验证明细与报表对应、汇总合计、退货链和失败隔离，默认忽略。没有真实数据时运行普通夹具测试，并在交付说明中记录真实批次未验证。
