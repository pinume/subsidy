#!/usr/bin/env python3
"""文档与代码一致性检查（CI 与 pre-commit 共用）。

1. 工作区现存的已跟踪 markdown（以及 AGENTS.md）中的相对链接都能打开，不使用 file:// 绝对链接。
2. 架构文档的「规则所有者」节点名的类型或函数在 src/ 中有定义，路径存在。
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
docs = {root / p for p in tracked if (root / p).is_file()}
agents = root / "AGENTS.md"
if agents.exists():
    docs.add(agents)

for doc in sorted(docs):
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

architecture = root / "docs/architecture.md"
if architecture.exists():
    sources = "\n".join(p.read_text(encoding="utf-8") for p in (root / "src").rglob("*.rs"))
    seams = re.search(r"^## 规则所有者.*?(?=^## |\Z)", architecture.read_text(encoding="utf-8"), re.S | re.M)
    for line in seams.group(0).splitlines() if seams else []:
        for token in re.findall(r"`([^`]+)`", line):
            if "/" in token:
                if not (root / token).exists():
                    errors.append(f"docs/architecture.md: 路径不存在 {token}")
            elif re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*", token):
                name = token.split("::")[-1]
                if not re.search(rf"\b(struct|enum|trait|fn|type)\s+{name}\b", sources):
                    errors.append(f"docs/architecture.md: src/ 中没有 {token} 的定义")

for error in errors:
    print(error)
sys.exit(1 if errors else 0)
