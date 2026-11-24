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
- Measured and inferred are kept apart in the README; every measurement names run, date and revision.
- No code fix in this hour unless trivial; a fix carries the smallest GPU-free Rust test, failing before and passing after.
- Disk: abort the build if free space on `/workspace` would drop below 8 GB; delete only scratch created by this run.
