# vNext architecture

The project is one `no_std` UEFI binary.

| Boundary | Owner | Current proof |
| --- | --- | --- |
| UEFI entry | `src/main.rs` | enter and remain alive |
| Debug output | `debug_line` | write one line to port `0xe9` |
| All other subsystems | deferred | add only with an acceptance test |

The v1 design, contracts, and decisions are archived in `v1_docs/` and are
not active requirements.
