# Blackwell (GB202) Windows Code 43 — resume, 2026-10-20

STATUS: LIVE, 2026-10-20. Resumes `traces/sweep_integrate_20261018` (FINAL_20261020.md,
blackwell_diff_20261020/, phase2_blackwell_probe_56830105/). Plan and every falsifier, written before
each run: `PLAN.md` (addenda are timestamped). Own lease 56909569, destroyed and API-absent
(`receipts.json`, `COST.md`: 4.756 USD).

## Setup (MEASURED)

GB202 / RTX 5090 (10de:2b85, subsys 1462:5301), KVM rental, host Linux 595.91.07 open. Native health
before any guest: host_preflight SOUND, `PROBE_DIE die=GB202`, v3 gates 9/9 + gate10 PASS, bare-metal
raw 30/30, CUDA 4/4 (`native/`). Generic LTSC 2024 install exit 0 (std VGA, 26 min); Windows 580.88 /
32.0.15.8088 / oem7.inf bound **disabled** (Code 22, both EnableGpuFirmware=1) at 16:05 UTC, one hour
after lease start (`preparation/`). Every arm: fresh child of the read-only parent (sha256 5667064…),
P19 flags (`box_scripts/p19.flags`), KF3_RPC_TRACE/BODY_TRACE, one `pnputil /enable-device`, ABI
580.65.06. Arm runner, exporter and probes: `box_scripts/`. Raw qemu logs, Windows bodies, images and
binaries stayed on the rental.

## Products (source → per-revision QEMU sha256, `preparation/bwr_binary_hashes.txt`)

| tag | source | contents | QEMU sha256 |
|---|---|---|---|
| base | 7f22d18df | frozen candidate-5 product | b33394dd… |
| cand | aa8fdea9c | draft PR53 (public CFB0/CEB0/D1B0 admission) | a73d922b… |
| fix1 | 844eb167f | cand + public NVCFFA_VIDEO_OFA admission | bc40f4b6… |
| fix1d | 5a072171d | fix1 + bounded arming-index log (DIAGNOSTIC, not for merge) | 2609e0d7… |
| fix2 | c58989dab | fix1 + §S.1 stub row for notifier 196 USE_GC6_REDUCED_THRESHOLD | 6886b2d7… |
| fix3 | ccb4b706c | fix2 + measured GB202/580 caps word 0x00c (tier 3, own die) | 8f35ade1… |
| fix3c | 0752115a7 | fix3 + KF3_DIAG_CAPS_PAGE_FILE caps-page override (DIAGNOSTIC, not for merge) | c90f3766… |

Product branch: `claude/blackwell-ofa-20261020` (stacked on PR53 aa8fdea9c, tests first each step).
Diagnostics: `claude/blackwell-diag-20261020`, `claude/blackwell-diag2-20261020`.

## Arms (MEASURED; `arms/<run>/`: api.tsv, summary.json, abort_context.txt, allocations.jsonl, events, diag)

| run(s) | n | Code | RPCs | abort (last non-teardown RPC: ordinal, selector, status) | unload | after |
|---|---|---|---|---|---|---|
| base_off_1, base_off_2b | 2 | 43 | 354 | 274 alloc CFB0 refused 0x56 (NotOnAllowlist) | 353 | — |
| cand_off_1/2 | 2 | 43 | 540 | 387 alloc CFFA refused 0x56 (CFB0 x2, CFB7 x6 accepted) | 539 | — |
| fix1_off_1/2, fix1d_off_1 | 3 | 43 | 595 | 416 EVENT_SET_NOTIFICATION, event 196 REPEAT, refused 0x56 | 594 | — |
| fix2_off_1/2, fix3_off_1 | 3 | **0** | 3832–3879 | Code 0 at +20 s; teardown ~3565–3612 | ~3831–3878 | bugcheck 0x116 VIDEO_TDR_FAILURE (P3 0xC000009A) ~40 s later |
| base_on_1/2, cand_on_1/2, fix2_on_1, fix3_on_1/2, trace_on_1, dtrace_on_1, caps_ctl_on | 10 | 43 | 384 | 275 4th CA7A cursor alloc (NV_OK), then teardown (SET_RMFREE_FLAGS + Free) | 383 | cand_on_1 only: bugcheck 0x1E after Code 43 (1/10) |
| capsdiag_on_1 (whole own-die caps page) | 1 | **0** | 5111 | — | 5110 | bugcheck 0x116 |
| caps_highb_on (own-die 0x680..0xfff) | 1 | **0** | 5017 | — | 5016 | bugcheck 0x116 |
| caps_lowa_on (own-die 0x000..0x624) / caps_pre_on (PRECOMP only) | 1+1 | 43 | 384 | 275 CA7A, as base | 383 | — |
| caps_post_on (POSTCOMP only) | 1 | — | 276 | no teardown | — | bugcheck 0x50 right after the 4th CA7A |

No System 4101 was readable in any arm (Code 43 arms: 0; Code-0 arms bugchecked before the probe).
Guest events (`arm_*_events.txt`): no nvlddmkm/Display/WUDF records in the Code 43 arms.

## Hypotheses (falsifiers stated in PLAN.md before each run)

- **H-NVDEC-OFF: SUPPORTED, not falsified.** PR53 accepts CFB0 and the guest progresses 186 RPCs to
  the next refused constructor (n=2 each arm, identical ordinals).
- **H-OFA-OFF: SUPPORTED.** CFFA admission (public resource_list.h:1877, same shape as admitted C9FA)
  moves the abort to the arming of notifier 196.
- **H-GC6-OFF: SUPPORTED.** With 196 ruled a §S.1 stub, OFF reaches **Code 0** (n=3). Root cause of the
  OFF Code 43 = three policy refusals in sequence: CFB0 constructor, CFFA constructor, notifier 196.
- **H-FN76 (fn76 0x20809064): FALSIFIED as a Code 43 cause.** Code 0 is reached with it still refused
  (12x per Code-0 arm; refusal source logged: `consttab cudart.0x20809064: unmeasured on GB202`).
- **H-ON-INDEPENDENT: CONFIRMED.** ON aborts after the 4th CA7A identically in base/cand/fix2/fix3.
- **H-CAPS-ON (word 0x00c alone): FALSIFIED** (fix3_on_1/2). **H-CAPS-ANY: SUPPORTED** (whole own-die
  page → Code 0); bisection: the decisive words are in public NVCA73 POSTCOMP_HDR_CAP*(i)
  (0x680+32i) and must be coherent with PRECOMP_WIN_PIPE_HDR_CAP*(i) (0x780+32i): POSTCOMP alone
  bugchecks 0x50, PRECOMP alone stays Code 43, both give Code 0. Control (derived page via the same
  hook) reproduces Code 43.
- H-ON-DISP (trapped BAR0 display access decides): not settled — trace_on_1 saw no trapped display
  access in the abort window, but untrapped user-area accesses are invisible to that tracer.

## CODE READ / INFERRED

- CODE READ: kf-disp's caps vocabulary (`crates/kf-disp/data/classes-580.65.06.tsv`) has no
  `NVCA73_POSTCOMP_HDR_*` row (0 of them; Ada's names are `POSTCOMP_HEAD_HDR_*`, 114 rows for C773),
  and `caps.rs` authors POSTCOMP words only under the `POSTCOMP_HEAD_HDR_*` names, so the Blackwell
  page carries no POSTCOMP word at all. INFERRED: this family naming gap is the ON root cause.
  Proposed fix (not made): derive the CA73 POSTCOMP_HDR_* vocabulary from public clca73.h and author
  those words by the same rules as Ada's POSTCOMP_HEAD_HDR_*, coherent with the PRECOMP rows; falsifier:
  ON still aborts after the 4th CA7A with the derived POSTCOMP words served. No captured page shipped.
- fn76 0x20809064: public category 0x90 = NV2080_CTRL_CLK | LEGACY_NON_PRIVILEGED (ctrl2080base.h:53);
  no public command/params in ogkm 580.65.06/580.159.04/595.84/610.57.04. kayfabe already answers this
  control from the host for authored requests (`kf_abi::gssreplay`); the Windows KMD's request words
  are not among its rows. Derivation path: measure the KMD's input words (bounded diagnostic log, as
  for the armings) and add a gssreplay row answered by the host for a request kayfabe authors — never
  the native 520-byte body.
- Next layer (OPEN): every Code-0 arm ends in bugcheck 0x116 VIDEO_TDR_FAILURE (status 0xC000009A)
  ~40 s after start; teardown follows a client's ZBC/0x20809064/PREEMPT_BIND probe sequence.
  Not investigated beyond `arms/fix2_off_*/kf_notable_lines.txt`.

## Not done

Native windows-on-vast reference on this board (time spent on the root-cause chain; the board passed
native Linux 595/580 health, and kayfabe reached Code 0 on it). Linux guest regression of fix1–fix3.
No merge; PR53/PR51/PR52 untouched except a factual comment on PR53.

## Phase 2 (lease 56939139, 19:13–20:36 UTC; stopped on the coordinator's wrap-up order)

See HANDOFF_CODEX.md for the full state. MEASURED (n=1 each, second GB202 board, parent with small-dump config):
- fix3_off_1: Code 0, then 0x116 (same parameters). Collection boot without kf3: System 1001 BugCheck
  0x116, minidump + 1 GB WATCHDOG LiveKernelReport, WER LiveKernelEvent **141 (VIDEO_ENGINE_TIMEOUT)** x3
  then 116; no 4101. Kernel GR channel dead at its report-semaphore segment; copy channel 0x80f GP_GET
  6877/7084 (identical to lease 1).
- fix4_off_1 (Blackwell 3D report semaphore admitted from NVIDIA open-gpu-doc): Code 0, GR channel alive
  (33 methods relayed), copy channel drained (19352 forwarded, GP_GET==PUT); **still 0x116** at ~+40 s.
  H-TDR-GRSEM: a real wall, not the last one (falsifier met for "sole cause").
- fix5_on_1 (CA73 POSTCOMP_HDR_CAPA derived): Code 43 at the same CA7A abort: H-POSTCOMP-DERIVED falsified.
- Native raw on lease 2: 29/30 while the Windows install ran beside it (concurrent-fuzz invariant
  violation), 30/30 + CUDA 4/4 when rerun alone (`lease2/native/`).
