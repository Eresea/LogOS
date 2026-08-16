# Development checks

Host scheduler checks:

```text
cargo fmt --check
cargo test --lib
cargo clippy --lib -- -D warnings
```

The complete host gate is:

```text
.\scripts\check.ps1 -Stage host
```

UEFI checks use `scripts/check.ps1`; the target is `x86_64-unknown-uefi` and the package has no
allocator. The bounded proof runner accepts `-Cpus 1`, `-Cpus 2`, or `-Cpus 8`:

```text
.\scripts\run.ps1 -Proof -Cpus 1 -TimeoutSeconds 60
```

Networking is enabled by default. The runner stages a bounded static-then-DHCP configuration and a
QEMU user-mode VirtIO-net device. Use `-NoNetwork` for an offline boot; it removes the profile and
keeps Network Disabled. Missing or malformed `NETWORK.CFG` also fails closed to Disabled:

```powershell
.\scripts\run.ps1 -Interactive -Cpus 1
.\scripts\run.ps1 -NoNetwork -Interactive -Cpus 1
.\scripts\run.ps1 -NoNetwork -Proof -Cpus 1
```

Network v1 host tests cover the fixed ABI, configuration fallback, socket generations, packet-page
copying, checksums, DHCP fallback, PCI capability parsing, and the bounded VirtIO queue model. The
real-peer QEMU network proof is a separate enabled-profile gate.

Proof mode captures debugcon output, rejects fatal markers, requires the structured PASS marker,
and terminates QEMU after the bounded timeout.
