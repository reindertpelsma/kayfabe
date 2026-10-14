# kf3 BAR0 trace mode — fast-guest evidence (2026-10-09)

**STATUS: LIVE, 2026-10-09.** Diagnostic-only mode (`docs/design/V3_BAR0_TRACE_MODE.md`, owner ruling
`docs/OWNER_RULINGS.md` §X). Binary: kf3 QEMU 10.2.4 built by `scripts/bench/build_kf3.sh` at
`9cebeea9` from a pristine `qemu-10.2.4.tar.xz` with `tools/vfio-gsp-observer/apply.py` applied
(sha256 in `qemu-sha256-9cebeea9.txt`); raw client and fast guest (host mode, kernel
7.0.0-34-generic) rebuilt from the same checkout. Bench host 172.22.1.20, RTX 4070 (AD104,
`0000:01:00.0`, nvidia bound, IOMMU group `DMA-FQ`), every run under
`/tmp/kayfabe-fastguest.lock` (taken per arm by `run_fast_guest.sh`).

| run | what | result |
|---|---|---|
| `rtoff` | 30-arm suite, mode OFF (no `KF3_BAR0_READ_TRACE`) | 30/30; `rtoff_suite.out` |
| `rton` | 30-arm suite, mode ON, `KF3_READ_TRACE_RANGES=all`, default caps, `-trace` with `scripts/bench/trace-events-vfio-reference.txt` | 30/30, 0 drops; `rton_suite.out`, `rton-per-arm.txt` |
| `rtcap` | `--ce-client-guest-ram`, ON, all ranges, `KF3_READ_TRACE_MAX_BYTES=4M` | PASS; cap hit, file 4 194 263 B of a 4 194 304 B cap, 2 996 553 records dropped and counted (`rtcap-rtobs-qemu-lines.txt`) |
| `rtobs` | `--engines`, ON (display + `0x0-0xfff,0x110000-0x110fff`), plus `x-gsp-observer` | PASS; 23 027 trace records, observer 644 records, 1 table attached, 0 gaps, 0 drops, `"device":"kf3-gpu"` (`rtobs-trace.log.gz`, `rtobs-gsp.jsonl.gz`) |

**Same tracer, same analysers.** The `rtobs` outputs go through the VFIO reference's tools unchanged:
`decode.py --jsonl --all-records` then `vfio_events.py` (`rtobs-vfio_events.txt`),
`mmio_window.py` (`rtobs-mmio_window-1-20.txt`), `vfio_msi_vectors.py` from
`claude/vfio-dvi-reference-20261008` (`rtobs-vfio_msi_vectors.txt`). A guest read of PMC_BOOT_0 is
the same text in both boots:
- VFIO boot3 (`traces/vfio_dvi_reference_20261008/boot3-qemu-trace.log.gz`, other branch):
  `vfio_region_read  (0000:01:00.0:region0+0x0, 4) = 0x194000a1`
- kf3 `rton` timer arm (`rton-timer-trace-head400.log`):
  `vfio_region_read  (0000:01:00.0:region0+0x0, 4) = 0x194000a1`

**Default off changes nothing.** `read_exits` from the drainer's mid-run heartbeat (only arms long
enough to print one): OFF 0 on all 6 such arms, as in suite `mchost2` (rev `0e64a960`, another
agent, 2026-10-08: 0 on all 7); ON 15 761 … 1 325 685. Per-arm wall seconds OFF vs `mchost2` agree
within ±1 s. No OFF log contains a trace line or report.

**Overhead with everything selected** (`rton` vs `rtoff`, the client's own clock): 65.6 s → 73.5 s
summed over 30 arms (+12 %); worst `ce-client-guest-ram` 19.8 s → 24.6 s (+24 %) while recording
2 165 647 lines (1 380 043 reads, 783 976 writes, 1 628 MSIs; 199.8 MB); 311 MB of trace for the
whole suite. The byte count is an upper bound of the file: equal in 29 arms, 7 bytes above it in one
(a timestamp without its microsecond fraction).

⊘ The status line's `read_exits` printed at realize is always 0; arms shorter than the 2 s heartbeat
print no later one, so `rton-per-arm.txt`/`rtoff-per-arm.txt` show 0 for them — the trace report's
counts are the measure there.
