#!/usr/bin/env bash
# Largest per-function stack frames in release service ELFs (prologue plus stack-probe
# totals: each function's own frame, not the call-chain peak). Build first with
# .\scripts\build-services.ps1 -Release, then run on base and branch and compare.
#
# Usage: stack-check.sh [tree-root] [image ...]   (defaults: git root; lockscreen atrium system)
root=${1:-$(git rev-parse --show-toplevel)}; shift || true
images=${*:-lockscreen atrium system}
for e in $images; do
  elf="$root/target/x86_64-unknown-none/release/logos-$e"
  [ -f "$elf" ] || { echo "== $e: missing $elf"; continue; }
  echo "== $e"
  objdump -d --no-show-raw-insn "$elf" | awk '
    /^[0-9a-f]+ <.*>:$/ { if (t >= 2048) print t, fn; fn = $2; n = 0; t = 0; l = 0; next }
    { n++ }
    n <= 14 && /sub +\$0x[0-9a-f]+,%r11/ { match($0, /\$0x[0-9a-f]+/); l = strtonum(substr($0, RSTART + 1, RLENGTH - 1)); t += l }
    n <= 16 && /sub +\$0x[0-9a-f]+,%rsp/ { match($0, /\$0x[0-9a-f]+/); v = strtonum(substr($0, RSTART + 1, RLENGTH - 1)); if (!(l > 0 && v == 4096)) t += v }
    END { if (t >= 2048) print t, fn }' | sort -rn | head -8
done
