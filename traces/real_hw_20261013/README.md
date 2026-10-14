# First real-hardware measurements of the Windows capture module's core (Linux twin), 2026-10-13

**STATUS: EVIDENCE, 2026-10-13.** Box: Vast contract 55810567 (KVM, RTX 4070 Ti SUPER = AD103, PCI 10de:2705, Linux NVIDIA driver 575.51.03 with GSP firmware 575.51.03 active, Ubuntu 6.8.0-59, `CONFIG_STRICT_DEVMEM=y`). The module's own library (`tools/windows-capture/lib`, branch `claude/windows-capture-module-20261013`, tree as of 23:00 CEST) ran on Linux with RAM read through `/proc/kcore` (read-only) and BAR0 through sysfs `resource0` mapped `PROT_READ`. Nothing was written to the device or to RAM. The capture file itself (37 MB, raw queue entries) stays host-only; only census TSVs are committed (the census never prints message bytes).

## Measured

1. **Mailbox pair (option a) is published only during a GSP boot.** At rest `0x110040/0x110044` read `0/0` (`mailbox_probe`, status 2). Polled at 20 Hz during repeated GSP boots (`mb1.txt`) it shows a non-zero pair for about 150 ms per boot (e.g. `0x6f105000 / 0x1` at t=19.99 s, back to `0/0` at 20.14 s), plus `0x80000000` during a suspend-like step. So option (a) needs roughly 100 Hz or faster polling; the engine, polling at about 1 kHz, attached through the mailbox (method 1) **12 times** in 80 s.
2. **The RAM scan (option b) finds the queues after the fact.** `scan_probe`: 13,197,438 pages (49 GB) in 14.5 s from userland, 2 signature hits, 1 bound: shared region at `0x12c400000`, 129 pages, command queue 262,144 B, status queue 262,144 B, both headers valid; the other hit (`0xbfee0000`) was refused by validation (false positive).
3. **The engine's live capture decodes.** `capture_probe` (the module's `kfwc_engine_step` loop against real memory, 80 s, driver unloaded and reloaded mid-run, repeated `nvidia-smi`): 7,551 records, 37,475,040 bytes, 12 attaches/12 detaches, 3,713 command + 3,772 status messages, 0 transport-seqNum gaps, 0 torn reads, 27 loss records, 27 resyncs (`cap1.out`). `kf-census` (branch `claude/gsp-census-20261013`, `e2a4618bb`, decode tag 580.82.07) decodes it: 116 distinct controls, 0 unanswered requests, 0 orphan replies; its oracle reports FAIL with 29 anomalies, exactly the engine's own accounting: 27 `LossUnknown` records and 2 captures that started late (first sequence 1133, not 0), about 2,269 messages missing in total (`census1.anomalies.tsv`). Ten of the 12 GSP boots were captured from sequence 0.
4. **Seven of the 40 "Windows-only" rows are also issued by the closed Linux driver** and answered by this AD103: `0x2080852a`, `0x2080852e`, `0x2080852f`, `0x2080853a`, `0x2080a618`, `0x20810108`, `0x20809064`, every call status 0. Their request and reply sizes equal the AD104/Windows 580.88 measurements in the inventory (1784, 528, 776, 1456, 8620, 992, 520 bytes). Values were NOT compared (only sizes). The other rows (`0x730122`, `0x730128`, `0x7302a3`, `0x7302a5`, `0x20802806`, `0x2081010d`, `0x00809908`, `0x20801111`) never appear on Linux: they are Windows-driver traffic.

## Not measured / caveats

- This is the Linux driver on an Ada GPU; the Windows driver's mailbox timing, `PAGE_READONLY` on a BAR mapping, physical==DMA addresses and the kernel module itself are still unverified on real hardware.
- The 12 attaches are an artefact of persistence mode being off (each `nvidia-smi` re-initialises the GPU); a steady state would show one attach.
- `scan_probe.c`/`capture_probe.c` are throwaway validation aids (kcore as the RAM source), not part of the module.
- The loss-record count is dominated by re-synchronisation after each detach; it is expected, not a defect, but the decoder's "FAIL" verdict will always trip on a capture that spans driver reloads.

## Control probe on the same AD103 (`probe_unpriv.txt`, `probe_admin.txt`), 2026-10-13

`kf-ctl-probe` (branch `claude/ctl-probe-20261013`) sent each listed control with ZEROED input to the real GPU, once as uid 65534 with no capabilities (`CapEff=0x0`: kayfabe's own situation) and once as root with `CAP_SYS_ADMIN` (`CapEff=0x1ffffffffff`, results labelled admin). Linux driver 575.51.03, no Xid logged in either run. Sizes are the reply sizes recorded in the Windows captures.

| result | controls |
|---|---|
| **OK unprivileged** (kayfabe can ask the host at runtime, tier 2) | `0x2080852a` (1784 B), `852e` (528), `852f` (776), `853a` (1456), `8524` (4), `9001` (8), `9009` (8), `a060` (28), `a088` (12), `a618` (8620), `a630` (1160), `a70a` (26), `a801` (1028), `b201` (2188) |
| **INSUFFICIENT_PERMISSIONS unprivileged, OK as root** (genuine `CAP_SYS_ADMIN` rows) | `0x20802806` (4 B), `0x20802801` (4 B), `0x20802a12` (8 B). Note `0x20802806` was listed "absent from every open table"; measured: it exists and is privileged. |
| **INVALID_PARAM_STRUCT** at the Windows size with zeroed input | `0x20809004` (1544 B), `0x20809064` (520 B). The Linux driver DID issue `0x20809064` with 520-byte requests and got OK, so zeroed input is invalid for these: they need the real request content (replay of a captured request), not a zero probe. |
| **NOT_SUPPORTED** | `0x2080a02f` (the real GPU also answers 0x56 to it, as in the Windows captures), `0x20800a3f`, `0x20800a80` (after INVALID_ARGUMENT on other sizes) |
| **INVALID_ARGUMENT at every size tried** (zero input is not enough; real inputs needed) | `0x2080853b`, `a619`, `0x2080121f`, `0x20800a2e`, `a87`, `ab8`, `b03`, `0x00809908` (NV0080) |
| **not probed in v1** | NV2081 `0x20810108`, `0x2081010d` (needs the binary-API object); NV0073 display controls (needs the display-common object) |

Reading: zero-input probing proves "answerable" but cannot prove "not answerable": every INVALID_* row needs a replay with the captured request. The 14 OK rows are strong evidence for the tier-2 route for the GSS-legacy family on this die/driver (Linux 575.51.03, AD103); the same probe at the host's own 595.91.07 is being run by the host agent and may differ. A probe is one die, one driver: nothing here is a per-die rule.
