# Blackwell (GB202) Windows Code 43 — resume plan and falsifiers

STATUS: LIVE, 2026-10-20 (written 15:40 UTC, BEFORE any product arm on own lease 56909569).

Resumes traces/sweep_integrate_20261018 (FINAL_20261020.md, blackwell_diff_20261020/RESULT.md,
phase2_blackwell_probe_56830105/RESULT.md). Products: baseline 7f22d18dfff8083345cdb54a4666018dca845384
(frozen candidate-5 product tree), candidate aa8fdea9cb0e00fd4e5ec148461d1e805d308e0c (draft PR53, three
code/test paths only). Same box, same frozen disabled-580.88 parent (bw580_bind.qcow2, sha256 recorded
on the rental), P19 flag set (p19.flags, 19 lines, copied from the lane), KF3_RPC_TRACE=1 and
KF3_RPC_BODY_TRACE=1 (diagnostic only), one pnputil enable per arm, guest ABI 580.65.06.

## Arms (interleaved, n=2 each)

base_off_1, cand_off_1, base_on_1, cand_on_1, base_off_2, cand_off_2, base_on_2, cand_on_2.
OFF = `display=off,gop=off,gop-efi=` (empty ROM), ON = default display/GOP.

## Hypotheses and falsifiers (stated before the runs)

- **H-NVDEC-OFF** (CFB0 refusal causes the OFF abort). CODE READ supporting fact: OFF ordinal 277 is the
  `0xcfb0` allocation refused 0x56 and ordinals 278.. are the >=20-Free teardown (lane api.tsv).
  *Falsifier:* in cand_off the CFB0 allocation is accepted (posted header status 0) and the guest still
  tears down immediately after it with Code 43 and no new RPC class/control after CFB0 (no progress).
  *Support:* cand_off accepts CFB0 and progresses past it (later abort or Code 0). A later abort supports
  only this wall.
- **H-ON-INDEPENDENT** (ON aborts before CFB0 for a different reason, after the 4th CA7A cursor
  allocation). *Falsifier:* cand_on behaves differently from base_on (abort ordinal or Code changes),
  which would mean CFB0 admission matters for ON too; or base_on requests CFB0.
- **H-FN76** (the fn76/0x20809064 refusal is causal). Already weakened (passing Ada tolerates 255 such
  refusals). *Falsifier:* the guest issues >=1 further non-Free RPC after the refusal in every arm (it
  continues). Public status CODE READ: category 0x90 = NV2080_CTRL_CLK | LEGACY_NON_PRIVILEGED
  (ctrl2080base.h:53, ogkm 580.65.06); no public command/params definition in ogkm 580.65.06,
  580.159.04, 595.84 or 610.57.04, so no reply can be authored from public definitions.

## Recorded per arm

Abort point (last non-Free RPC before the first run of >=20 Free, ordinal/function/selector/status),
unload (fn 47) ordinal, Code/problem, 4101 count, System/Application events (nvlddmkm, Display,
Kernel-PnP, DriverFrameworks-UserMode, WER), posted allocation headers (public rpc_gsp_rm_alloc_v03_00
class/status/paramsSize), kayfabe host log lines (non-body) from 15 RPCs before teardown to the unload.
Raw bodies, qemu logs, images and binaries stay on the rental.
