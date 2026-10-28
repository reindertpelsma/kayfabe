# BW11 - GB202 completion probe, rebuilt-parent run (lease 57799086)

STATUS: 2026-10-26, RESEARCH. Outcome: **INCONCLUSIVE** - the 0x141 / 0x113 of BW6 did not
reproduce, so no causal claim about H1-T, H1-G, H2 or H3-H7 is made. Label: rebuilt-parent probe.

All claims below are tagged with the run they come from (lease 57799086, the private capture under
`[PRIVATE_PATH]`, hashes in `private_sha256.txt`). Raw logs stay private.

## What ran

MEASURED (lease 57799086, box `[PRIVATE_PATH]` logs, captured in `bw11_capture.tgz`): one RTX 5090 (GB202,
PCI 10de:2b85, board SUBSYS 205710DE) in Japan, machine 138291, 15 visible cores, host driver
595.91.07. Box pipeline of BW6 (box gates `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS`, bare suite
29/29, Windows 11 LTSC eval install, 580.88 bind, sealed parent) with the fix binary built from
007ba1eb03db9e21d351e465e95b46cbc3134a95 (QEMU sha256
05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448, equal to BW5/BW6).
Rebuilt parent `bw580_bind.qcow2` sha256 51b5afda321b4e4b7f4ea2420575fc1e52a4c414bf8db4eb8cbd15be81a90fd0.
Differences from BW6 (lease 57556167): other machine/host/board, fresh Windows install and secrets,
native gr-tier fixture / PRECOMP read / native fuzz omitted (`controller.diff`).

## Arms (both: default p19 flags, one enable at about 40 s guest uptime, no application)

| arm | KF3_COMPLETION_PROBE | Code 0 | hold after Code 0 | Xid | System 1001/41/6008/4101 |
| --- | --- | --- | --- | --- | --- |
| on_fix (accidentally without the flag; harness defect, see below) | unset | 10:58:33Z | 601 s | 0 | none |
| on_fix_probe | 1000 | 11:16:21Z (early observer) | 600 s | 0 | none |

MEASURED (2026-10-26, lease 57799086, RTX 5090, `arm_on_fix.log`, `arm_on_fix_probe.log`, `collect_*.txt`): both arms ended
with ARM_HOLD_OBSERVED, rc 0 for the first arm, no GUEST_PANICKED, no XID_STOP, zero host Xid lines,
and no 1001/41/6008/4101/4117/4119 or nvlddmkm/Display error event in the guest System log. The
only guest events of note are WER LiveKernelEvent P1=1a8 and 1b8 (display), present in BW6 too.

## Probe lines of the probe arm (Translated CE4 ring, token 0x80f, engine 0xd)

MEASURED (2026-10-26, lease 57799086, `on_fix_probe_b1_qemu.log`, sha256 in `private_sha256.txt`): the log has the
line "kf3: PROBE on - completion probe, overdue after 1000 ms"; 18 per-fence PROBE lines (8 for
token 0x80f, seq 1..8, every host fence complete within 34-184 us of submission, guest USERD
GP_PUT == GP_GET each time), 51 PROBE-DUMP records, 5 PROBE-RETIRE lines.
MEASURED (same log): all 47 PROBE-DUMP records of token 0x80f are
GUEST-SILENT-AFTER-COMPLETION (idle ring), in all 47 the guest USERD GP_PUT equals GP_GET (last
value 3044 after a wrap, host ring fence counter 2097), none shows a fence still in flight, none
reports NOT-LANDED; probe lines span 629 s of host monotonic time.
MEASURED (same log): `releases=[]` on all 18 per-fence lines, i.e. this probe decoded no guest
semaphore release for these rings in the healthy run, so a "matching release" witness could not be
produced even on a good run.
MEASURED (lease 57799086, `on_fix_b1_qemu.log` of the no-probe arm): token 0x80f RETIRED with
last_put=1740 and gp_get=1740 (BW6 failing run: last_put=4676, gp_get=3360).

## Pre-registered hypotheses

MEASURED (2026-10-26, lease 57799086, `on_fix_probe_b1_qemu.log`): this passing run shows the same RUSD refusals (`GSP REFUSED fn76/0x20800afe=0x56`, `0x20800aff=0x56`) and the same display LiveKernelEvents 1a8/1b8 as BW6, so neither is sufficient for the failure on this machine (H5, H7; interaction untested).

UNRESOLVED / INCONCLUSIVE for all of H1-T, H1-G, H2 delivery, H3-H7: no 0x141 reproduced, so the
falsifiers have no failing event to be applied to. The healthy-run probe data above is a baseline
only (host fences complete, guest GP_GET follows GP_PUT) and does not test the failing path.

## Harness defects found (not product events)

1. `b1n_arms.py prepare` rewrites `[PRIVATE_PATH]<run>.sh` from the plan, so the first arm ran
   without the flag; the plan got a second arm entry whose only difference is the flag
   (`b1n_arms.diff`). 2. The secondary Xid guard held the old authority file and killed the first
   probe-arm launch before the enable (no guest GPU work); restarted with the updated arms list.
3. The RPC timing observer asserted on a leftover output file and exited (rc 18 of arm.sh); the
   collection boot was run by hand. None changes the product binary, flags or sealed parent.

## Teardown

MEASURED (2026-10-26, vastai show instances-v1 at 11:31:47Z): instance 57799086 destroyed by exact id
(destroy issued 11:31:27Z), API absence verified twice; private capture sha256-verified before
destroy (`private_sha256.txt`).
