#!/usr/bin/env python3
"""Validate Cargo package edges and service-boundary assembly use."""

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ROLES = {
    "logos-abi": (0, "contracts"),
    "logos-core": (0, "core"),
    "logos-net": (2, "network-protocol"),
    "logos-remote": (1, "remote-contracts"),
    "logos-network-service": (2, "network-service"),
    "logos-service-rt": (1, "service-rt"),
    "logos-store": (2, "store"),
    "logos-storage-service": (3, "storage"),
    "logos-terminal": (3, "terminal"),
    "logos-terminal-service": (3, "terminal-service"),
    "logos-sessions-service": (3, "sessions-service"),
    "logos-gateway-service": (3, "gateway-service"),
    "logos-uefi": (0, "uefi-boot"),
    "logos-test": (99, "test"),
    "logosctl": (99, "host-client"),
}

ALLOWED = {
    "logos-abi": set(),
    "logos-core": {"logos-abi"},
    "logos-net": set(),
    "logos-remote": {"logos-abi"},
    "logos-network-service": {"logos-abi", "logos-net", "logos-service-rt"},
    "logos-service-rt": {"logos-abi"},
    "logos-store": {"logos-abi"},
    "logos-storage-service": {"logos-abi", "logos-service-rt", "logos-store"},
    "logos-terminal": set(),
    "logos-terminal-service": {"logos-abi", "logos-service-rt", "logos-terminal"},
    "logos-sessions-service": {"logos-abi", "logos-service-rt"},
    "logos-gateway-service": {"logos-abi", "logos-remote", "logos-service-rt"},
    # The boot adapter is the documented temporary exception while terminal
    # bootstrap remains statically linked.
    "logos-uefi": {"logos-abi", "logos-core", "logos-terminal", "logos-remote"},
    "logos-test": {"logos-abi", "logos-store", "logos-terminal", "logos-remote"},
    "logosctl": {"logos-remote"},
}

# Dependencies flow toward lower rings. Boot/test adapters are explicit
# exceptions because they assemble outer services without owning them.
RING_EXCEPTIONS = {
    ("logos-uefi", "logos-terminal"),
    ("logos-uefi", "logos-remote"),
}


def metadata() -> dict:
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=ROOT,
            text=True,
        )
    )


def package_edges(data: dict) -> dict[str, set[str]]:
    edges = {}
    for package in data["packages"]:
        edges[package["name"]] = {
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency.get("path") is not None and dependency["name"] in ROLES
        }
    return edges


def violations(data: dict) -> list[str]:
    edges = package_edges(data)
    errors = []
    for package, dependencies in edges.items():
        unexpected = dependencies - ALLOWED.get(package, set())
        for dependency in sorted(unexpected):
            errors.append(f"{package} depends on unapproved internal package {dependency}")
        package_ring = ROLES.get(package, (99, "unknown"))[0]
        for dependency in sorted(dependencies):
            dependency_ring = ROLES.get(dependency, (99, "unknown"))[0]
            if (
                dependency_ring < 99
                and package_ring < 99
                and dependency_ring > package_ring
                and (package, dependency) not in RING_EXCEPTIONS
            ):
                errors.append(
                    f"{package} ring {package_ring} imports outer {dependency} ring {dependency_ring}"
                )
    for package in (
        "logos-network-service",
        "logos-terminal-service",
        "logos-sessions-service",
        "logos-storage-service",
        "logos-gateway-service",
    ):
        source = ROOT / "crates" / package / "src" / "main.rs"
        text = source.read_text(encoding="utf-8")
        if any(token in text for token in ("asm!", "core::arch", "int 0x80")):
            errors.append(f"{source.relative_to(ROOT)} uses assembly outside logos-service-rt")
    return errors


def dot(data: dict) -> str:
    edges = package_edges(data)
    lines = ["digraph LogOS {", "  rankdir=LR;"]
    for package in sorted(edges):
        label = ROLES.get(package, (99, "unknown"))[1]
        lines.append(f'  "{package}" [label="{package}\\n{label}"];')
    for package, dependencies in sorted(edges.items()):
        for dependency in sorted(dependencies):
            lines.append(f'  "{package}" -> "{dependency}";')
    lines.append("}")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--output", "-o")
    args = parser.parse_args()
    data = metadata()
    errors = violations(data)
    output = dot(data)
    if args.output:
        Path(args.output).write_text(output, encoding="utf-8")
    elif not args.check:
        print(output, end="")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
