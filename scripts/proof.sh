#!/usr/bin/env bash
# Headless Linux QEMU proof runner: the boot-marker subset of scripts/run.ps1 -Proof.
# Usage: scripts/proof.sh [-c CPUS] [-t TIMEOUT_SECONDS] [-f EXTRA_KERNEL_FEATURES]
# Needs: x86_64-unknown-uefi + x86_64-unknown-none targets, qemu-system-x86_64, OVMF.
set -euo pipefail

cpus=1
timeout=90
extra=""
while getopts "c:t:f:" opt; do
    case "$opt" in
        c) cpus="$OPTARG" ;;
        t) timeout="$OPTARG" ;;
        f) extra=",$OPTARG" ;;
        *) echo "usage: $0 [-c cpus] [-t timeout] [-f features]" >&2; exit 2 ;;
    esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
target="$root/target"
ovmf="${OVMF_CODE:-}"
if [[ -z "$ovmf" ]]; then
    for candidate in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/qemu/OVMF.fd; do
        [[ -f "$candidate" ]] && ovmf="$candidate" && break
    done
fi
[[ -f "$ovmf" ]] || { echo "Set OVMF_CODE to an OVMF firmware file." >&2; exit 1; }

cd "$root"
cargo build --release --target x86_64-unknown-uefi --features "qemu-proof$extra"

export CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS='-C opt-level=z -C relocation-model=static -C code-model=large -C link-arg=--image-base=0x10000000000 -C link-arg=--no-pie'
cargo build --release --target x86_64-unknown-none -p logos-service-images --bins --features qemu-proof
cargo build --release --target x86_64-unknown-none -p logos-service-images --bin logos-user --features user-kdf

esp="$target/esp-linux"
rm -rf "$esp"
mkdir -p "$esp/EFI/BOOT" "$esp/EFI/LOGOS"
cp "$target/x86_64-unknown-uefi/release/logos-vnext.efi" "$esp/EFI/BOOT/BOOTX64.EFI"
for name in INPUT DISPLAY TERMINAL SESSION FLOW STORAGE NETWORK FETCH DEVICE USER SHELL LOCKSCREEN ATRIUM SYSTEM; do
    src="$target/x86_64-unknown-none/release/logos-${name,,}"
    dst="$esp/EFI/LOGOS/$name.ELF"
    cp "$src" "$dst"
    if (( $(stat -c %s "$dst") > 2048 * 1024 )); then
        objcopy --strip-all --strip-section-headers "$dst"
    fi
    (( $(stat -c %s "$dst") <= 2048 * 1024 )) || { echo "$name exceeds 2048 KiB" >&2; exit 1; }
done

disk="$target/proof-storage-linux.raw"
if [[ ! -f "$disk" ]]; then
    truncate -s 64M "$disk"
    printf 'LOGOSBLK' | dd of="$disk" conv=notrunc status=none
fi

log="$target/qemu-proof-linux-$cpus.log"
rm -f "$log"
qemu-system-x86_64 -machine q35 -m 256M -smp "$cpus" \
    -drive "if=pflash,format=raw,readonly=on,file=$ovmf" \
    -drive "format=raw,file=fat:rw:$esp" \
    -drive "if=none,id=storage-disk,format=raw,file=$disk,cache=writethrough" \
    -device virtio-blk-pci,drive=storage-disk,disable-legacy=on \
    -device VGA,id=video0,xres=1280,yres=800 \
    -display none -no-reboot \
    -debugcon "file:$log" -global isa-debugcon.iobase=0xe9 &
qemu=$!
trap 'kill $qemu 2>/dev/null || true' EXIT

deadline=$((SECONDS + timeout))
status=1
while (( SECONDS < deadline )); do
    if [[ -f "$log" ]]; then
        if grep -aq 'LogOS vNext: QEMU proof PASS' "$log"; then status=0; break; fi
        if grep -aEiq 'LogOS vNext: (FATAL|QEMU proof FAIL|panic)|FAULT' "$log"; then break; fi
    fi
    kill -0 $qemu 2>/dev/null || break
    sleep 0.5
done
cat "$log" 2>/dev/null || true
if (( status == 0 )); then echo "PROOF PASS (-smp $cpus)"; else echo "PROOF FAIL (-smp $cpus); log: $log" >&2; fi
exit $status
