# vNext architecture

The vNext kernel is one UEFI package with one privileged entry point.

| Boundary | Current owner | Contract |
| --- | --- | --- |
| UEFI entry | `src/main.rs` | enter, print one debug line, remain alive |
| Debug output | `debug_line` | bytes to port `0xe9`, followed by CRLF |
| Everything else | deferred | added only with a passing acceptance proof |

The kernel remains `no_std`, fixed-resource, and observable through the QEMU
debug console. There are no service crates, allocators, runtimes, or public ABI
modules in this milestone.
