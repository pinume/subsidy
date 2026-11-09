#!/usr/bin/env bash
# 逐字节比较两次运行的 国补明细/ 与 国补报表/ 输出（解压 xlsx 后比较，忽略记录生成时间的 docProps/core.xml）。
# 用法：scripts/compare-outputs.sh <旧运行根目录> <新运行根目录>
# 退出码：0 全部一致；1 有差异（逐个列出文件）。
set -euo pipefail
old=$1 new=$2
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
status=0
for dir in 国补明细 国补报表; do
  for file in "$new/$dir"/*.xlsx; do
    name=$dir/$(basename "$file")
    if [[ ! -f "$old/$name" ]]; then
      echo "仅新运行存在：$name"; status=1; continue
    fi
    rm -rf "$work/a" "$work/b"
    unzip -q "$old/$name" -d "$work/a"
    unzip -q "$file" -d "$work/b"
    if ! diff -rq -x core.xml "$work/a" "$work/b" >/dev/null; then
      echo "不一致：$name"; status=1
    fi
  done
done
[[ $status -eq 0 ]] && echo "全部一致"
exit $status
