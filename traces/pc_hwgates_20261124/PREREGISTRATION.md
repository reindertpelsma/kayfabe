# Preregistration, written before any run (pc_hwgates_20261124)

STATUS: LIVE, 2026-11-24. Design intent and falsifiers only; no result here.

Run: kf3 built from the head of `origin/next` (revision `63e460694`, recorded again by `build_kf3.sh`) on the owner's pc,
RTX 4070, host driver 595.91.07, no GPU bind (kf3 reaches the host GPU through unprivileged RM; the desktop stays up).

## Claims and falsifiers

C1. The nine harness gates (`scripts/bench/v3_gates.sh`) still pass on this GPU with the batch candidate (ogkm-derived ingress
allowlist, channel-link gate, access-right derivation, debugger-session caps, unmapped-reads-zero default ON, walk/lease/wake fixes).
Falsifier: any `GATEn_VERDICT` other than PASS, a missing gate, or a refusal line
(`PRIVILEGED CHANNEL REFUSED`, `CHANNEL BIRTH REFUSED`, `CHANNEL CLASS REFUSED`, `CUDA THREAD REFUSED`) in the log.

C2. All 30 fast-suite arms pass through kf3 with a Linux guest on this host (`KF_DEVICE=kf3 fast_suite.sh pc-hwgates-<rev> 180`).
Falsifier: any arm that is not PASS. Not a failure of the claim but a finding: an arm that is SKIP.

C3. The new ingress allowlist refuses nothing that a supported guest driver needs. Falsifier: a fast-suite row, or the Windows
driver, fails AND the kf3/QEMU log of that boot names a refusal (grep `refus|REFUSED|denied|not allowed|allowlist|ingress`
in `fast_*_qemu.log`, the serial log, and the Windows boot's QEMU log). A failure with no named refusal is reported as a failure,
not attributed to the allowlist.

C4. (only if >= 20 minutes remain) The non-proprietary Windows guest (`win11_base`) boots on kf3, the NVIDIA driver loads
without Code 43 and `nvidia-smi` in the guest shows the RTX 4070. Falsifier: guest does not come up, Code 43 / no adapter,
or `nvidia-smi` in the guest does not list the 4070. S3 sleep/wake arm B is attempted only if time remains and is not part of the claim.

C5. After the run the host is idle: no qemu process, lock released, `PC_ACTIVE.log` has a matching END line, the host driver is
still 595.91.07 and loaded, no new Xid in host dmesg. Falsifier: any of these differs.

## Rules for the run
- Measured and inferred are kept apart in the README; every measurement there names its run, date and revision (pc run 2026-11-24, `63e46069`).
- No code fix in this hour unless trivial; a fix carries the smallest GPU-free Rust test, failing before and passing after.
- Disk: abort the build if free space on `/workspace` would drop below 8 GB; delete only scratch created by this run.

## Addendum 1, written 2026-11-24 ~18:30Z after the fast suite (28/30) and the first Windows boot, before the controls

Seen so far (pc run of 2026-11-24, `63e46069`, not yet explained): fast-suite arms `--defer-liveness` and `--missing-page-fault` FAIL on `63e46069`; the first
Windows kf3 boot (default flags) had the guest answer ssh, then the host RM logged `mmuWalkMap: Failed to map VA Range
0x120000000 to 0x20f9fffff. Status = 0x51` (NV_ERR_NO_MEMORY) and QEMU was gone before the first guest GPU query returned.

H1. The two fast-suite failures are caused by `unmapped-reads-zero=on` (#580): with `KF3_DEV_EXTRA=unmapped-reads-zero=off` both arms PASS
on the same binary, and the pre-batch `ed41beea` binary passes both with defaults. Falsifier: either arm still fails with the property
off, or `ed41beea` fails it. (Run before this addendum; result in the README.)

H2. The Windows host-RM `NV_ERR_NO_MEMORY` and the QEMU death are caused by the same property (the fill maps zero pages over a
multi-GB unmapped range, which needs host page-table memory): with `KF_DEV_EXTRA=unmapped-reads-zero=off` on `63e46069` the guest
boots, `nvidia-smi` lists the 4070 and the host log has no `mmuWalkMap ... 0x51`. Falsifier: the same `0x51` / death with the property
off, or no `0x51` / death on a repeat with defaults (then the first boot was not reproducible and is reported as 1 of 2).
Second control: `ed41beea` (before #545, #580), defaults. Neither is a claim about the allowlist; C3 stays open for Windows until
a boot with a working driver names a refusal or not.
