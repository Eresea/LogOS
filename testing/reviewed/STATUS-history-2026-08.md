# Test status history — 2026-08

This reviewed record preserves the mixed-era evidence removed from the active status ledger.

## Earlier catalog snapshot

The earlier fixed-seed catalog contained 84 proofs: 51 ready and 33 intentionally skipped. The
mixed-era suite totals were:

| Suite | Passed | Failed | Skipped |
| --- | ---: | ---: | ---: |
| core | 1 | 0 | 9 |
| console | 12 | 0 | 3 |
| platform | 16 | 0 | 16 |
| persistence | 8 | 0 | 0 |
| network | 9 | 3 | 0 |
| remote | 0 | 8 | 0 |
| main | 44 | 12 | 28 |

Those totals are historical and are not current proof results.

## Superseded failures and claims

Earlier Remote runs timed out waiting for `LogOS: Gateway started`; the harness now uses structured
readiness and host-side authority. The five unfinished Remote scenarios are intentionally skipped,
not partially passed. Earlier raw-DHCP and Gateway-start claims remain useful phase evidence but are
not the current direct-client baseline.
