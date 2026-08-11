#!/usr/bin/env python3
"""Check that vNext stays a single package."""

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
    )
)
packages = [package["name"] for package in metadata["packages"]]
if packages != ["logos-vnext"]:
    raise SystemExit(f"expected one package, found {packages}")
