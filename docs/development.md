# Development checks

Host checks:

```text
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The complete host gate is:

```text
.\scripts\check.ps1 -Stage host
```

`check.ps1`, `verify.ps1` and `program-proof.ps1` wrap native commands in `Invoke-Native`
(`scripts/native.ps1`) and exit non-zero on the first failing command; `verify.ps1 -Proof` runs all
CPU counts and then fails if any failed.

UEFI checks use `scripts/check.ps1`; the target is `x86_64-unknown-uefi` and the package has no
allocator. The bounded regression proof runner accepts `-Cpus 1`, `-Cpus 2`, or `-Cpus 8`:

```text
.\scripts\run.ps1 -Proof -Cpus 1 -TimeoutSeconds 60
```

`scripts/run.ps1` uses the optimized release kernel by default; pass `-Debug` when an
unoptimized kernel is specifically needed. Networking is enabled by default for normal boots.
Regression proof runs are offline unless
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

Interactive runs use QEMU's SDL display so relative mouse capture recenters
the host pointer after the click; `Ctrl+Alt+G` toggles the grab.

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

## Parser hardening

Code that consumes untrusted bytes has property tests (`proptest`, dev-dependency only) next to the
host tests, so `cargo test --workspace` runs them:

| Parser | Tests |
| --- | --- |
| Package manifests and headers (`package/`) | `package/tests/parser_props.rs` |
| ABI message decoders and `wire_enums_valid` guards (`abi/`) | `abi/tests/message_props.rs`, `abi/tests/wire_guards.rs` |
| Storage journal, copy-on-write and catalog formats (`storage/`) | `storage/tests/format_props.rs` |
| Network frames, requests and checksums (`network/`) | `network/tests/packet_props.rs` |
| ELF plan, loader and page tables (`src/`) | `src/elf_props.rs` (in-crate: the host frame pool only exists under the library's own `cfg(test)`) |

Each asserts that arbitrary bytes never panic, that valid encodings round-trip, and that mutated
valid inputs either still validate consistently or are rejected. `wire_guards.rs` uses a
deterministic xorshift loop instead of proptest so it also runs under Miri, where an enum field a
guard forgot to check is reported as undefined behaviour. Proptest suites are skipped under Miri.
A failing case is saved to a `proptest-regressions` file; commit it with the fix.

Long fuzz runs use cargo-fuzz. `fuzz/` is excluded from the workspace and is not part of CI:

```text
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run package_decode -- -max_total_time=300
```

Targets are `package_decode`, `abi_messages`, `storage_volumes`, `network_frames` and `elf_plan`.
Crashing inputs land in `fuzz/artifacts/<target>/`; reproduce with
`cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>` and turn the input into a unit or
property regression test with the fix. Corpora live in `fuzz/corpus/` and are not committed.

## Miri and Linux boot proof (CI)

`.github/workflows/hardening.yml` runs two jobs on pull requests and `main`:

- Miri (`cargo +nightly miri test`) over the host-tested crates. Run it locally with
  `rustup toolchain install nightly --component miri`; the package list and the excluded tests (with
  the reason for each) are in the workflow.
- A headless Linux QEMU boot proof, `scripts/proof.sh -c 1 -t 600`, which needs `qemu-system-x86`,
  `ovmf` and the `x86_64-unknown-uefi` and `x86_64-unknown-none` targets.
