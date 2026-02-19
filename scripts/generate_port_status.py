#!/usr/bin/env python
from __future__ import annotations

import ast
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    py_file = root / "main.py"
    out_file = root / "PORTING_STATUS.md"
    src_dir = root / "src"

    tree = ast.parse(py_file.read_text(encoding="utf-8"))
    top_level_funcs: list[str] = []
    classes: dict[str, list[str]] = {}

    for node in tree.body:
        if isinstance(node, ast.FunctionDef):
            top_level_funcs.append(node.name)
        elif isinstance(node, ast.ClassDef):
            methods = [
                n.name
                for n in node.body
                if isinstance(n, ast.FunctionDef)
                and not (n.name.startswith("__") and n.name.endswith("__"))
            ]
            classes[node.name] = methods

    rust_code = ""
    for path in src_dir.glob("*.rs"):
        rust_code += path.read_text(encoding="utf-8", errors="ignore")
        rust_code += "\n"

    def status_for(name: str) -> str:
        needle = f"fn {name}"
        return "Ported" if needle in rust_code else "Pending"

    lines: list[str] = []
    lines.append("# Native Rust Port Status")
    lines.append("")
    lines.append("Auto-generated from `main.py` symbols vs `src/*.rs` function names.")
    lines.append("")
    lines.append("## Top-level functions")
    lines.append("")
    lines.append("| Function | Status |")
    lines.append("|---|---|")
    for fn in top_level_funcs:
        lines.append(f"| `{fn}` | {status_for(fn)} |")

    lines.append("")
    lines.append("## Class methods")
    lines.append("")
    for cls, methods in classes.items():
        lines.append(f"### `{cls}`")
        lines.append("")
        lines.append("| Method | Status |")
        lines.append("|---|---|")
        if not methods:
            lines.append("| *(no methods)* | - |")
        for m in methods:
            lines.append(f"| `{m}` | {status_for(m)} |")
        lines.append("")

    out_file.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Wrote {out_file}")


if __name__ == "__main__":
    main()
