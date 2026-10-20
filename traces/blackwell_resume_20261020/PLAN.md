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

## Addendum 16:35 UTC — stated BEFORE the fix1 arms (no fix1 arm had started)

MEASURED so far (n=1 each): base_off_1 aborts at CFB0 (ordinal 274, refused NotOnAllowlist 0x56),
cand_off_1 accepts CFB0 x2 and CFB7 x6 and aborts at the next refused constructor, NVCFFA_VIDEO_OFA
(ordinal 387, 0x56); both ON arms abort after the 4th CA7A (ordinal 275), identical for base and cand.

- **H-OFA-OFF** (CFFA refusal is the next OFF wall). Product fix1 = 844eb167f (branch
  claude/blackwell-ofa-20261020, PR53 + public CFFA admission, tests first). *Falsifier:* in fix1_off
  CFFA is accepted and the guest tears down immediately after it (no new non-Free RPC) with Code 43.
  *Support:* progress past CFFA (a later abort or Code 0).

## Addendum 16:52 UTC — before fix1d_off_1 and trace_on_1 (neither started)

MEASURED: fix1_off_1 accepts CFFA (ordinal 387) and progresses to 595 RPCs; new abort = the 13th
consecutive NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION (0x20800301) at ordinal 416, refused 0x56 by
kayfabe's notifier admission (inittables EventSetNotification), then teardown, unload 594, Code 43.
H-OFA-OFF supported (not falsified). The refused event index is not in the logs.

- fix1d (5a072171d, branch claude/blackwell-diag-20261020, NOT for merge) = fix1 + a bounded (64-line)
  log of each arming's event/action words. Measurement only: which index is refused.
- **H-ON-DISP** (ON abort after the 4th CA7A is decided from display-plane/BAR0 state, not from an RPC
  reply). trace_on_1 = base product ON with WINVM_TRACE=1 KF3_BAR0_READ_TRACE=1 (display-range reads,
  all writes). *Falsifier:* between the doorbell of the 4th CA7A allocation and the doorbell of the
  first teardown RPC the trace holds no display-range (0x610000..0x6fffff) access at all.

## Addendum 17:18 UTC — before the fix2 arms (none started)

MEASURED fix1d_off_1 (diagnostic log): armings in order 194, 35, then 44, 43, 113, 120, 4, 33, 139,
157, 197, 122, 158, 2 (all NV_OK), then **196 = NV2080_NOTIFIERS_USE_GC6_REDUCED_THRESHOLD, REPEAT,
refused 0x56 (86)**, then the 12 served ones are disabled (action 0) and teardown follows. Ada's
Windows order (kf-rm tests/code43_startdevice_batch.rs) has CE3 (26) in that position and never arms 196.

- **H-GC6-OFF** (the 196 refusal is the next OFF wall). Product fix2 = c58989dab (claude/blackwell-ofa-20261020:
  PR53 + CFFA + a §S.1 stub row for 196, tests first). *Falsifier:* fix2_off serves the 196 arming (0x0)
  and the guest still tears down right after it with Code 43 (no further non-Free RPC beyond the
  arming sequence). *Support:* progress past it (a later abort or Code 0).

## Addendum 17:52 UTC — before dtrace_on_1 (not started)

MEASURED: fix2_off_1/2 reach **Code 0** (Status OK 20 s after enable), then a guest bugcheck 0x116
VIDEO_TDR_FAILURE (P3 0xC000009A) ~40 s later; fix2_on_1 unchanged (abort after the 4th CA7A, Code 43).
trace_on_1: no trapped display-range BAR0 access between the last CA7A doorbell and the teardown
doorbell, only MMU-invalidate writes (the tracer cannot see untrapped display user-area accesses, so
H-ON-DISP is NOT settled by it).

- **H-ON-METHODS** (after the cursor allocations the guest pushes display methods whose completion
  kf-disp does not deliver as the Blackwell driver expects). dtrace_on_1 = base ON with
  KF3_DISPLAY_TRACE=1 KF3_DISPLAY_METHOD_TRACE=1 (diagnostic logs only). *Falsifier:* no display
  method/update/notifier activity is logged between the 4th CA7A allocation and the first teardown RPC.

## Addendum 17:55 UTC (box clock) — before the fix3 arms (none started)

MEASURED dtrace_on_1 (KF3_DISPLAY_TRACE/METHOD_TRACE): the guest pushes **no display methods at all**
in the ON run (no method record), and kf-disp logs at realize: `NO MEASURED caps word 0x00c for die
arch 0x1b0 impl 0x2, driver branch 580: the authored page stays clean and a Windows guest's display
may not come up`. MEASURED caps page of this GB202 (root, read-only BAR0, tools/read_display_caps_page.py):
0x00c = 0x00000102 under Linux 580.159.04 and under 595.91.07 (127 words identical), caps/.

- **H-CAPS-ON** (the unmeasured caps word 0x00c makes the Blackwell Windows driver abandon display
  init after the cursor channels). Product fix3 = fix2 + one measured GB202/580 row in
  crates/kf-disp/data/caps_measured.tsv (tier 3, OWNER_RULINGS §AF/§AH; tests first).
  *Falsifier:* fix3_on still aborts right after the 4th CA7A (ordinal 275/276) with Code 43.

## Addendum 18:22 UTC — after fix3_on_1, before capsdiag_on_1 (not started)

MEASURED fix3_on_1: the GB202 caps word 0x00c is now served (kf-disp: 1 T3 measured, 0 unmeasured)
and ON still aborts after the 4th CA7A (ordinal 275, Code 43): **H-CAPS-ON falsified** (n=1 so far).
The served page has 40 derived words; this die's own read has 127 non-zero words (126 differ).

- **H-CAPS-ANY** (some other caps-page word decides the ON abort). capsdiag_on_1 = fix3 + DIAGNOSTIC
  branch claude/blackwell-diag2-20261020 (0752115a7, NOT for merge) with KF3_DIAG_CAPS_PAGE_FILE = this
  box's own Linux 580.159.04 read (oracle only, never shipped). *Falsifier:* with the full own-die
  page served, ON still aborts right after the 4th CA7A with Code 43.

## Addendum 18:42 UTC — after capsdiag_on_1, before the caps bisection arms (none started)

MEASURED capsdiag_on_1: with the whole own-die page (127 words) served, **ON reaches Code 0**
(5111 RPCs) and then bugchecks like the OFF Code-0 arms: H-CAPS-ANY supported, H-CAPS-ON (0x00c alone)
falsified (fix3_on_1/2, n=2). fix3_off_1: Code 0 then bugcheck (as fix2).

Bisection pages (caps/*.txt; derived fix3 words with this die's own words overlaid in one region):
- caps_lowA_on: native words in 0x000..0x624 (core/head/SOR region), rest derived. *Falsifier for
  "the needed words are in lowA":* Code 43 with the CA7A abort.
- caps_highB_on: native words in 0x680..0xfff (per-head and window precomp caps), rest derived.
  *Falsifier for "the needed words are in highB":* Code 43 with the CA7A abort.
- caps_ctl_on (control): the derived page alone through the same hook. *Expected:* Code 43 (else the
  hook itself changes behaviour and every caps arm is void).

## Addendum 18:46 UTC — after caps_lowa_on / caps_highb_on, before caps_post_on / caps_pre_on

MEASURED: caps_lowa_on (native 0x000..0x624) **Code 43** (CA7A abort); caps_highb_on (native
0x680..0xfff) **Code 0**. So the decisive words are in public NVCA73 POSTCOMP_HDR_CAP*(i) (0x680+32i)
and/or PRECOMP_WIN_PIPE_HDR_CAP*(i) (0x780+32i) (clca73.h, ogkm 580.65.06).
- caps_post_on: native POSTCOMP only. *Falsifier for "POSTCOMP decides":* Code 43.
- caps_pre_on: native PRECOMP only. *Falsifier for "PRECOMP decides":* Code 43.
