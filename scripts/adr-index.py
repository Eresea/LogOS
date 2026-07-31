#!/usr/bin/env python3
"""Validate that docs/adr/README.md indexes every ADR with its status."""

from pathlib import Path
import argparse
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
ADR_DIR = ROOT / "docs" / "adr"
INDEX = ADR_DIR / "README.md"
ADR = re.compile(r"^(\d{4})-[^/]+\.md$")
ROW = re.compile(r"^\| \[(\d{4})\]\(([^)]+)\) \| ([^|]+) \|")


def expected() -> list[tuple[str, str, str]]:
    records = []
    for path in sorted(ADR_DIR.iterdir()):
        match = ADR.match(path.name)
        if not match:
            continue
        text = path.read_text(encoding="utf-8")
        status = re.search(r"^- Status: (.+)$", text, re.MULTILINE)
        if status is None:
            raise ValueError(f"{path.relative_to(ROOT)} has no status")
        records.append((match.group(1), path.name, status.group(1).strip()))
    return records


def actual() -> list[tuple[str, str, str]]:
    records = []
    for line in INDEX.read_text(encoding="utf-8").splitlines():
        match = ROW.match(line)
        if match:
            records.append(tuple(part.strip() for part in match.groups()))
    return records


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        wanted = expected()
        found = actual()
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    if wanted != found:
        print("docs/adr/README.md is out of date", file=sys.stderr)
        print(f"expected: {wanted}", file=sys.stderr)
        print(f"found:    {found}", file=sys.stderr)
        return 1
    if not args.check:
        print("ADR index is current")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
