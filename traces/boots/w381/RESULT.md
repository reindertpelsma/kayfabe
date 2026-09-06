# w381 — the guest-servable probe: the differential, as it was measured

**STATUS — 2026-09-06 — LIVE.** Design and reasoning: `docs/design/w381_the_guest_servable_probe.md`.
This directory is the **durable artefact**; the rented box it was measured on is not.

Bench: vast instance `50072086`, RTX 3060 (GA106), host driver **580.159.04 open module**,
guest kernel `6.8.0-138-generic`, guest driver 580.159.04, QEMU 10.2.4 + the QOM shim.
Provisioned from `scripts/bench/provision_{box,host_driver,bench_tree}.sh` in ~12 minutes.
⊘ Destroyed after the run.

| arm | source | verdict line |
|---|---|---|
| native / launch-dma  | `6bff78df` | `W381_TABLE_ROW arm=native probe=launch-dma  pass=7 fail=0 notrun=0 seen=7 of=7` |
| native / sem-release | `6bff78df` | `W381_TABLE_ROW arm=native probe=sem-release pass=7 fail=0 notrun=0 seen=7 of=7` |
| **guest / launch-dma** | `6bff78df` | `W381_TABLE_ROW arm=guest  probe=launch-dma  pass=6 fail=1 notrun=0 seen=7 of=7` |

`w381_logs.tgz` (49 KB) holds, verbatim:

| file | what it is |
|---|---|
| `run_w381_ndma2_native.log`  | native arm, `--w381` (launch-dma), **7/7** |
| `run_w381_nsem2_native.log`  | native arm, sem-release, **7/7** |
| `run_w381_guest3_probe.log`  | ★ the **guest** arm at the fix — 6/7, the battery's own output verbatim |
| `run_w381_guest3_qemu.log`   | the device's stderr for that boot (`doorbells=324 … Ce=324 … unrouted=0`) |
| `run_w381_guest3_dmesg.log`  | the **guest driver's** own ring buffer for that boot |
| `run_w381_guest_probe.log`   | ⊘ the **FIRST** guest run, `pass=4 fail=2` — the run that found the 33rd-submission wall |
| `run_w381_guest2_probe.log`  | ⊘ `--map-stress` alone, pre-fix: `releases 32/186`, first failure at cycle 9 slot 2 |

★ The two pre-fix logs are kept deliberately. `releases 32/186` with `placements exact 48/48`
and `VAs recovered 44/44` beside it is what identified the defect as the **probe's own
`GP_PUT` arithmetic** rather than the mapping plane's — and it is the concrete instance of
*"a guest-only red cannot distinguish 'we are broken' from 'the probe is wrong'"*.
