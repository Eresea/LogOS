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
allocator. The bounded regression proof runner accepts `-Cpus 1`, `-Cpus 2`, or `-Cpus 8`:

```text
.\scripts\run.ps1 -Proof -Cpus 1 -TimeoutSeconds 60
```

Networking is enabled by default for normal boots. Regression proof runs are offline unless
`-NetworkProof` is explicitly supplied. The enabled Network proof currently remains a post-merge
TCP follow-up; DHCP fallback proof is also deferred. Use `-NoNetwork` for any offline boot; it
removes the profile and keeps Network Disabled. Missing, malformed, or oversized `NETWORK.CFG`
files also fail closed to Disabled:

```powershell
.\scripts\run.ps1 -Interactive -Cpus 1
.\scripts\run.ps1 -NoNetwork -Interactive -Cpus 1
.\scripts\run.ps1 -Proof -Cpus 1
.\scripts\run.ps1 -Proof -NetworkProof -Cpus 1
.\scripts\run.ps1 -Proof -VirtioGpu -Cpus 1 -QmpPort 4450
```

`-VirtioGpu` replaces the proof VGA device with QEMU's VirtIO-GPU device and
exercises the Core-owned scanout and hardware cursor path; the proof checks
cursor-plane publication and movement markers because QEMU framebuffer dumps
do not include the separate hardware cursor plane. Without it, the existing
VGA proof path remains unchanged.

Network v1 host tests cover the fixed ABI, configuration fallback, socket generations, packet-page
copying, checksums, DHCP fallback, PCI capability parsing, and the bounded VirtIO queue model. The
real-peer QEMU network proof is a separate enabled-profile gate.

Proof mode captures debugcon output, rejects fatal markers, requires the structured PASS marker,
and terminates QEMU after the bounded timeout.

The filesystem package proof seeds a fresh v3 disk with a real service ELF, activates it through the
internal Core hook, checks corrupt-package rollback, and boots again to prove package reopen:

```powershell
.\scripts\package-proof.ps1 -Release
```

Each offline package boot has a ten-second maximum; the broader scheduler proof remains a
separate gate.

The v5 storage proof attaches a fresh VirtIO disk, validates the live v5 root and User catalog
system-pool placement, corrupts the inactive root, and boots again to prove torn-root recovery:

```powershell
.\scripts\storage-proof.ps1 -ResetDisk
```
