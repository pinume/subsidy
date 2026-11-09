#!/usr/bin/env python3
"""文档与代码一致性检查（CI 与 pre-commit 共用）。

1. 已跟踪的 markdown（以及本地 AGENTS.md）中的相对链接都能打开，不使用 file:// 绝对链接。
2. 本地 AGENTS.md 存在时，其 Seams 一节点名的类型或函数在 src/ 中有定义，路径存在。
"""
import re
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
errors = []

tracked = subprocess.run(
    ["git", "ls-files", "*.md"], cwd=root, capture_output=True, text=True, check=True
).stdout.split()
docs = [root / p for p in tracked]
agents = root / "AGENTS.md"
if agents.exists():
    docs.append(agents)

for doc in docs:
    text = doc.read_text(encoding="utf-8")
    for target in re.findall(r"\]\(([^)\s]+)\)", text):
        if target.startswith("file://"):
            errors.append(f"{doc.relative_to(root)}: 绝对链接 {target}，改用相对路径")
            continue
        if re.match(r"[a-z]+:", target) or target.startswith("#"):
            continue
        path = target.split("#", 1)[0]
        if path and not (doc.parent / path).exists():
            errors.append(f"{doc.relative_to(root)}: 链接不存在 {target}")

if agents.exists():
    sources = "\n".join(p.read_text(encoding="utf-8") for p in (root / "src").rglob("*.rs"))
    seams = re.search(r"^## Seams.*?(?=^## |\Z)", agents.read_text(encoding="utf-8"), re.S | re.M)
    for line in seams.group(0).splitlines() if seams else []:
        for token in re.findall(r"`([^`]+)`", line):
            if "/" in token:
                if not (root / token).exists():
                    errors.append(f"AGENTS.md: 路径不存在 {token}")
            elif re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*", token):
                name = token.split("::")[-1]
                if not re.search(rf"\b(struct|enum|trait|fn|type)\s+{name}\b", sources):
                    errors.append(f"AGENTS.md: src/ 中没有 {token} 的定义")

for error in errors:
    print(error)
sys.exit(1 if errors else 0)
