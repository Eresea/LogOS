#!/usr/bin/env python3
"""Generate GraphViz DOT of LogOS internal module dependencies.

Usage:
    python3 scripts/arch-deps.py | dot -Tsvg > arch.svg
    python3 scripts/arch-deps.py --check  # Exit non-zero on ring violations
"""

import re
import sys
from pathlib import Path
from dataclasses import dataclass
from typing import Dict, List, Set, Optional
from enum import Enum

class Ring(Enum):
    CORE = 0
    FOUNDATION = 1
    SYSTEM = 2
    SESSIONS = 3
    RUNTIME = 4
    EXPERIENCE = 5
    UNKNOWN = 99

# Module -> Ring mapping
MODULE_RINGS: Dict[str, Ring] = {
    # Ring 0 - Core
    "main": Ring.CORE,
    "scheduler": Ring.CORE,
    "memory": Ring.CORE,
    "capabilities": Ring.CORE,
    "interrupts": Ring.CORE,
    "ipc": Ring.CORE,
    "virtual_memory": Ring.CORE,
    "acpi": Ring.CORE,
    "pci": Ring.CORE,
    "health": Ring.CORE,
    "trace": Ring.CORE,
    "debug": Ring.CORE,
    "services": Ring.CORE,  # registry only
    "boot_info": Ring.CORE,
    
    # Ring 1 - Foundation
    "display": Ring.FOUNDATION,
    "input": Ring.FOUNDATION,
    "text": Ring.FOUNDATION,
    "virtio": Ring.FOUNDATION,
    "keyboard": Ring.FOUNDATION,
    "console": Ring.FOUNDATION,  # recovery console
    
    # Ring 2 - System (future)
    # "supervisor": Ring.SYSTEM,
    # "identity": Ring.SYSTEM,
    # "secrets": Ring.SYSTEM,
    # "time": Ring.SYSTEM,
    # "store": Ring.SYSTEM,
    # "network": Ring.SYSTEM,
    # "audit": Ring.SYSTEM,
    # "update": Ring.SYSTEM,
    
    # Ring 3 - Sessions
    "terminal": Ring.SESSIONS,
    "commands": Ring.SESSIONS,
    "mode": Ring.SESSIONS,
    
    # Ring 4 - Runtime (future)
    # "wasm": Ring.RUNTIME,
    # "package": Ring.RUNTIME,
    # "app": Ring.RUNTIME,
    # "workspace": Ring.RUNTIME,
    # "tools": Ring.RUNTIME,
    
    # Ring 5 - Experience (future)
    # "compositor": Ring.EXPERIENCE,
    # "shell": Ring.EXPERIENCE,
}

# Known external crates (stdlib, uefi, etc.) - ignore in analysis
EXTERNAL_CRATES = {
    "core", "alloc", "compiler_builtins", "uefi", "x86_64", 
    "bitflags", "log", "spin", "volatile", "raw_cpuid"
}

@dataclass
class ModuleInfo:
    name: str
    ring: Ring
    submodules: List[str]
    uses: List[str]  # crate::module::item references

def extract_mods(content: str) -> List[str]:
    """Extract `mod foo;` declarations."""
    return re.findall(r'^\s*mod\s+(\w+)\s*;', content, re.MULTILINE)

def extract_uses(content: str) -> List[str]:
    """Extract `use crate::module::...` references."""
    # Matches: use crate::module::... or crate::module::...
    pattern = r'(?:use\s+)?crate::(\w+)::'
    return list(set(re.findall(pattern, content)))

def extract_external_uses(content: str) -> List[str]:
    """Extract `use external_crate::...` references."""
    pattern = r'use\s+([a-zA-Z_][a-zA-Z0-9_-]*)::'
    return re.findall(pattern, content)

def get_module_ring(module_name: str) -> Ring:
    """Determine ring for a module."""
    return MODULE_RINGS.get(module_name, Ring.UNKNOWN)

def analyze_source(src_dir: Path) -> Dict[str, ModuleInfo]:
    """Analyze all .rs files in src/."""
    modules = {}
    
    for rs_file in src_dir.glob("*.rs"):
        if rs_file.name == "main.rs":
            module_name = "main"
        else:
            module_name = rs_file.stem
        
        content = rs_file.read_text()
        submods = extract_mods(content)
        uses = extract_uses(content)
        
        ring = get_module_ring(module_name)
        
        modules[module_name] = ModuleInfo(
            name=module_name,
            ring=ring,
            submodules=submods,
            uses=uses
        )
    
    return modules

def check_ring_violations(modules: Dict[str, ModuleInfo]) -> List[str]:
    """Check for inward dependency violations."""
    violations = []
    
    for mod_name, info in modules.items():
        if info.ring == Ring.UNKNOWN:
            continue
            
        for use_mod in info.uses:
            if use_mod in EXTERNAL_CRATES:
                continue
            if use_mod not in modules:
                continue
                
            used_info = modules[use_mod]
            if used_info.ring == Ring.UNKNOWN:
                continue
            
            # Violation: inner ring (lower number) depends on outer ring (higher number)
            # Core(0) should not depend on Foundation(1), etc.
            if used_info.ring.value > info.ring.value:
                violations.append(
                    f"RING VIOLATION: {mod_name} (Ring {info.ring.value}) "
                    f"depends on {use_mod} (Ring {used_info.ring.value})"
                )
    
    return violations

def generate_dot(modules: Dict[str, ModuleInfo], show_external: bool = False) -> str:
    """Generate GraphViz DOT output."""
    lines = [
        "digraph LogOS {",
        "  rankdir=LR;",
        "  node [fontname=\"JetBrains Mono\", fontsize=10];",
        "  edge [fontname=\"JetBrains Mono\", fontsize=9];",
        "",
        "  // Ring subgraphs (visual grouping)",
    ]
    
    # Group by ring
    rings_order = [Ring.CORE, Ring.FOUNDATION, Ring.SYSTEM, Ring.SESSIONS, Ring.RUNTIME, Ring.EXPERIENCE]
    ring_colors = {
        Ring.CORE: "#1a1a2e",
        Ring.FOUNDATION: "#16213e",
        Ring.SYSTEM: "#0f3460",
        Ring.SESSIONS: "#533483",
        Ring.RUNTIME: "#e94560",
        Ring.EXPERIENCE: "#ff6b6b",
    }
    
    for ring in rings_order:
        ring_modules = [m for m in modules.values() if m.ring == ring]
        if not ring_modules:
            continue
        
        color = ring_colors.get(ring, "#ffffff")
        lines.append(f'  subgraph cluster_{ring.name.lower()} {{')
        lines.append(f'    label = "Ring {ring.value} — {ring.name}";')
        lines.append(f'    style = filled;')
        lines.append(f'    color = "{color}";')
        lines.append(f'    fontcolor = "#ffffff";')
        for m in ring_modules:
            lines.append(f'    "{m.name}";')
        lines.append("  }")
        lines.append("")
    
    # Module nodes with ring-based styling
    for info in modules.values():
        if info.ring == Ring.UNKNOWN:
            lines.append(f'  "{info.name}" [shape=box, style=dashed, color=gray];')
        else:
            color = ring_colors.get(info.ring, "#ffffff")
            lines.append(f'  "{info.name}" [shape=box, style=filled, fillcolor="{color}", fontcolor=white];')
    
    # Edges: submodule (dashed), uses (solid)
    for info in modules.values():
        for sub in info.submodules:
            if sub in modules:
                lines.append(f'  "{info.name}" -> "{sub}" [style=dashed, color=gray, label="mod"];')
        
        for use_mod in info.uses:
            if use_mod in EXTERNAL_CRATES:
                if show_external:
                    lines.append(f'  "{info.name}" -> "{use_mod}" [style=dotted, color=blue, label="ext"];')
            elif use_mod in modules:
                used_ring = modules[use_mod].ring
                # Color edges by violation status
                if used_ring.value > info.ring.value:
                    lines.append(f'  "{info.name}" -> "{use_mod}" [color=red, penwidth=2, label="VIOLATION"];')
                elif used_ring.value < info.ring.value:
                    lines.append(f'  "{info.name}" -> "{use_mod}" [color=green];')
                else:
                    lines.append(f'  "{info.name}" -> "{use_mod}" [color=yellow];')
    
    lines.append("}")
    return "\n".join(lines)

def main():
    import argparse
    
    parser = argparse.ArgumentParser(description="LogOS architecture dependency analyzer")
    parser.add_argument("--src", default="src", help="Source directory")
    parser.add_argument("--check", action="store_true", help="Exit non-zero on ring violations")
    parser.add_argument("--show-external", action="store_true", help="Show external crate dependencies")
    parser.add_argument("--output", "-o", help="Output file (default: stdout)")
    args = parser.parse_args()
    
    src_dir = Path(args.src)
    if not src_dir.exists():
        print(f"Error: Source directory '{src_dir}' not found", file=sys.stderr)
        sys.exit(1)
    
    modules = analyze_source(src_dir)
    violations = check_ring_violations(modules)
    
    dot_output = generate_dot(modules, show_external=args.show_external)
    
    if args.output:
        Path(args.output).write_text(dot_output)
    else:
        print(dot_output)
    
    if violations:
        print("\n# RING VIOLATIONS DETECTED:", file=sys.stderr)
        for v in violations:
            print(f"#   {v}", file=sys.stderr)
    
    if args.check and violations:
        sys.exit(1)
    
    if violations:
        sys.exit(2)  # Violations but not --check mode

if __name__ == "__main__":
    main()