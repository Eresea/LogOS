#!/usr/bin/env python3
"""Check repository-relative Markdown links without external tooling."""

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)")


def exact_path(path: Path) -> bool:
    try:
        parts = path.relative_to(ROOT).parts
    except ValueError:
        return False
    current = ROOT
    for part in parts:
        match = next((item for item in current.iterdir() if item.name == part), None)
        if match is None:
            return False
        current = match
    return True


def main() -> int:
    failures = []
    for document in sorted(ROOT.rglob("*.md")):
        if any(part in {"review", "reviewed"} for part in document.relative_to(ROOT).parts):
            continue
        for match in LINK.finditer(document.read_text(encoding="utf-8")):
            target = match.group(1)
            if target.startswith(("http://", "https://", "mailto:", "#")):
                continue
            if target.startswith("file:"):
                failures.append(f"{document.relative_to(ROOT)}: absolute file link {target}")
                continue
            relative = target.split("#", 1)[0]
            if not relative:
                continue
            candidate = (document.parent / relative).resolve()
            if not exact_path(candidate):
                failures.append(f"{document.relative_to(ROOT)}: missing link {target}")
    for failure in failures:
        print(failure, file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
