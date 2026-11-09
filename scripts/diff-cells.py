"""按单元格比较两次运行的输出，列出差异行与前几处差异单元格。

用法：uv run -q --with openpyxl python -I scripts/diff-cells.py <旧运行根目录> <新运行根目录> [每个文件最多列出的行数，默认5]
先用 scripts/compare-outputs.sh 找出不一致的文件，再用本脚本看具体单元格。
"""
import sys
from pathlib import Path

import openpyxl

old_root, new_root = Path(sys.argv[1]), Path(sys.argv[2])
limit = int(sys.argv[3]) if len(sys.argv) > 3 else 5

for folder in ("国补明细", "国补报表"):
    for old_file in sorted((old_root / folder).glob("*.xlsx")):
        new_file = new_root / folder / old_file.name
        old_book = openpyxl.load_workbook(old_file, read_only=True)
        new_book = openpyxl.load_workbook(new_file, read_only=True)
        changed, shown = 0, []
        for old_sheet, new_sheet in zip(old_book.worksheets, new_book.worksheets):
            old_rows = list(old_sheet.iter_rows(values_only=True))
            new_rows = list(new_sheet.iter_rows(values_only=True))
            if len(old_rows) != len(new_rows):
                shown.append(f"  [{old_sheet.title}] 行数 {len(old_rows)} → {len(new_rows)}")
            for index, (old_row, new_row) in enumerate(zip(old_rows, new_rows), start=1):
                if old_row == new_row:
                    continue
                changed += 1
                if len(shown) < limit:
                    cells = [
                        f"{openpyxl.utils.get_column_letter(col)}: {a!r} → {b!r}"
                        for col, (a, b) in enumerate(zip(old_row, new_row), start=1)
                        if a != b
                    ]
                    shown.append(f"  [{old_sheet.title}] 第{index}行 " + "；".join(cells[:3]))
        if changed or shown:
            print(f"{folder}/{old_file.name}：{changed} 行不同")
            print("\n".join(shown))
