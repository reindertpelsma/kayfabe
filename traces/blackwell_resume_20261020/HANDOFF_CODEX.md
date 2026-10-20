# Handoff to Codex — Blackwell (GB202) Windows 580.88 under kayfabe

STATUS: LIVE, 2026-10-20 ~20:30 UTC. Written on the coordinator's wrap-up order (owner's Claude limit).
Read with README.md (lease 1 results), PLAN.md (every hypothesis + falsifier, timestamped), COST.md,
receipts.json. Lease-2 evidence: `lease2/`.

## 1. Branches and revisions (all pushed, nothing merged)

| branch | head | what |
|---|---|---|
| claude/blackwell-resume-20261020 | (this commit) | evidence only (traces/blackwell_resume_20261020) |
| claude/blackwell-ofa-20261020 | see `git ls-remote`; product commits below | stacked on draft PR53 aa8fdea9c |
| claude/blackwell-diag-20261020 | 5a072171d | DIAGNOSTIC: bounded log of EVENT_SET_NOTIFICATION event/action (not for merge) |
| claude/blackwell-diag2-20261020 | 0752115a7 | DIAGNOSTIC: KF3_DIAG_CAPS_PAGE_FILE caps-page override (not for merge) |

Product commits on claude/blackwell-ofa-20261020 (each: test EXPECTED RED first, then fix; >=2 killing
tests per mutant; clippy clean on changed lines; rustfmt edition 2024):
- 844eb167f fix1: public NVCFFA_VIDEO_OFA admission (kf-abi capability, since 570.86.15).
- c58989dab fix2: notifier 196 USE_GC6_REDUCED_THRESHOLD as a §S.1 Stub row (kf-abi eventnotify) — OWNER REVIEW (§6).
- ccb4b706c fix3: measured GB202/580 caps word 0x00c = 0x102 (kf-disp caps_measured.tsv, own-die tier 3).
- 71588cecb fix4: Blackwell 3D SET_REPORT_SEMAPHORE_A..D admitted (kf-chan grtables), source NVIDIA open-gpu-doc @9fdf5c40 clcd97.h/clce97.h — OWNER REVIEW: new public reference not pinned in third_party/. (95b7b6ff6 adds a 2nd killing test.)
- 048c28b9c fix5: CA73 POSTCOMP_HDR_CAPA derivation (kf-disp; derive spec + 5 regenerated classes-*.tsv, +101 rows each, nothing else changed; caps_family_pages.tsv +4 words 0x007b0000).
- ace26f23f: owner-review comment on the 196 row.
QEMU sha256 per rev: preparation/bwr_binary_hashes.txt (lease 1); lease-2 builds: fix3 ccb4b706, fix4 71588cec, fix5 048c28b9 (box-built, hashes in /root/prov on the destroyed box; not exported).

## 2. Measured results (GB202 RTX 5090, Windows 580.88/32.0.15.8088, ABI 580.65.06, host 595.91.07, P19 flags)

Display OFF (n per row; ordinals = RPC ordinals in the responder trace):
- base 7f22d18df: Code 43, CFB0 alloc refused 0x56 @274, unload 353 (n=2).
- PR53 aa8fdea9c: Code 43, CFFA refused @387, unload 539 (n=2).
- fix1: Code 43, 13th EVENT_SET_NOTIFICATION (event 196, REPEAT) refused @416, unload 594 (n=3).
- fix2/fix3: **Code 0**, then bugcheck 0x116 VIDEO_TDR_FAILURE (P3 0xC000009A, P4 4) ~40 s later (n=3 lease 1 + n=1 lease 2, different board).
- fix4: **Code 0**, still 0x116 (n=1, lease2/arms/fix4_off_1) — but see §3: the GR channel no longer dies and the copy channel drains.
Display ON: base/PR53/fix2/fix3 abort after the 4th CA7A cursor alloc @275 (n=10, Code 43). Whole own-die
caps page (diag) → Code 0 then 0x116; bisection: native POSTCOMP+PRECOMP region → Code 0; POSTCOMP alone →
bugcheck 0x50; PRECOMP alone → Code 43; control (derived page via hook) → Code 43.
fix5_on_1 (POSTCOMP derivation, lease 2): **Code 43, same abort after the 4th CA7A (@275, 384 RPCs)**;
kf-disp served 45 words (44 rule, 1 measured). H-POSTCOMP-DERIVED FALSIFIED (n=1): the six NVKMS-asserted
CAPA fields alone are not what the Windows driver needs; the decisive native words are elsewhere in
0x680..0x87c (bisection), i.e. widths/CAPB..G and/or PRECOMP coherence (§4).

## 3. TDR 0x116 facts

- Lease 2 crash collection (boot without kf3, collect.sh): System 1001 BugCheck 0x116 (0xffffc883b8ac2010,
  0xfffff803287b4930, 0xffffffffc000009a, 4); minidump C:\Windows\Minidump\102026-59031-01.dmp (4.38 MB,
  sha256 0ac322c2…); LiveKernelReports\WATCHDOG-20261020-2007.dmp (1 GB); Application WER: three
  LiveKernelEvent **141 VIDEO_ENGINE_TIMEOUT_DETECTED** then BlueScreen 116. No System 4101 (the guest
  bugchecks before it is written). Dumps stayed on the rental (destroyed); not analysed (no symbols).
- fix2/fix3 (lease 1 x3, lease 2 x1, identical numbers): the kernel GR channel (token 0x3, runlist 0) is
  marked dead at gp 1: `ring REFUSED ... GrMethod { class 0xCE97, method 0x1B00,
  "SET_REPORT_SEMAPHORE_* (this family's public 3D header does not define it)" }`; the async-copy kernel
  channel token 0x80f (runlist 1) retires with forwarded=7084, last_put=7084, **gp_get=6877**.
- fix4 (report semaphore admitted): GR channel 0x3 dead=None, gp_get=last_put=2, 33 GR methods relayed;
  copy channel 0x80f forwarded=19352, submissions=1296, last_put=gp_get=2968 at retire (drained); NSI
  relays for GR0 and CE4 fire. **Still 0x116** at ~+40 s, RPCs 3965, teardown @3700 after
  0x20801211 (PREEMPT_BIND probe) and 0x20801111 (SW runlist) controls; 0x20809064 still refused 5x near the
  end. So H-TDR-GRSEM is a real wall but NOT the last one: the remaining engine timeout is unidentified.
- Not a user-ring lost interrupt as far as measured (stalls were kernel channels); the relay-branch arm
  (claude/relay-simple-20261020 @aaceb73cb + KF3_DIAG_PROGRESS=1000) was NOT run — that head is not a
  descendant of 7f22d18df (45 commits, 128 files differ), so stacking needs a merge first.
- Next steps: at the 0x116 under fix4, list every channel's last_put/gp_get at TDR time (not only at
  retire), the engine of the 0x141 (the LiveKernel dump P2/P3 are pointers only), and whether a
  GR/compute/copy semaphore acquire is pending; compare with native trace
  (traces/sweep_integrate_20261018/phase2_windows_gb202_56816449/native_trace) around the end of init.

## 4. POSTCOMP derivation (display on)

Public facts: clca73.h (ogkm 580.65.06) names `NVCA73_POSTCOMP_HDR_CAP{A..G}(i)` at 0x680+32i (per output
TILE); Turing..Ada name `POSTCOMP_HEAD_HDR_CAP*`. Public NVKMS nvkms-evo4.c:1826-1834 reads
POSTCOMP_HDR_CAPA(tile) for each usable tile (SYS_CAPC_TILE_EXISTS) and asserts OCSC0/OCSC1/HCLPF/OSCAN/
DTH/DSC present; SCLR+VFILTER -> tile type 0. kayfabe's derive spec lacked the CA73 name, so the page had
no POSTCOMP word.
Coded (fix5 048c28b9c): derive spec + regenerated vocab; caps::page sets those six fields per virtual
tile (0x007b0000), SCLR/VFILTER absent, widths unclaimed. Not coded: FULL_WIDTH/UNIT_WIDTH (native
16/14; NVKMS asserts UNIT_WIDTH==16 only on C5), CAPB OLUT (CA73 renamed OLUT_LOGSZ->OLUT_SZ 12:1;
native 0x0006aa0x; sdr_page skips OLUT on physical-address families), PRECOMP coherence (POSTCOMP-only
native bugchecked 0x50). fix5_on WAS still Code 43 (§2). Next single variable: add CAPB OLUT with SZ in the kayfabe engine's
units (check nvkms-evo4 OLUT parsing for SZ semantics) as the next single variable; falsifier as in PLAN.

## 5. Re-creating a lease fast (what worked, traps)

- Offers: `vastai search offers 'vms_enabled=true num_gpus=1 gpu_name=RTX_5090' -o dph_total --raw`;
  offer ids change between searches — select by machine_id (UK machine 33614 at 0.909 $/h, Taiwan
  38450 at 1.135). Image `docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16`, `--disk 180`.
- Arm the timer first: `/root/night/arm_autodestroy.sh <id> <min>`; record id in receipts.json.
- Scripts: box_scripts/ (ctl.sh provision→gates→tree→native→install→bind; arm.sh; seq.sh; collect.sh;
  fixbuild.sh <name> <sha> <branch>; q3.sh). ~45 min lease→first arm (install 26 min).
- Traps hit: (1) replacing arm.sh while a seq runs breaks bash (seq now copies it per run);
  (2) `win_vm.sh stop` returns before QEMU exits — wait for pgrep to clear (bind and the arm guard raced);
  (3) `pkill -f <name>` over ssh kills your own ssh shell; (4) native raw concurrent-fuzz failed 1/30 while
  the Windows install ran beside it (rerun alone 30/30); (5) GPU BDF differs per box (07.0 vs 08.0);
  (6) set -e + empty grep in the hold block aborted export (fix4 exported by hand).

## 6. Owner questions

1. Notifier 196 USE_GC6_REDUCED_THRESHOLD as a §S.1 silent stub (agent's reading; PSTATE_CHANGE precedent;
   counter-reading §S.3 "power events"). Refusing it keeps GB202 Windows at Code 43.
2. NVIDIA open-gpu-doc @9fdf5c40 as a public source for Blackwell 3D methods (fix4); pin under third_party?
3. fn76 0x20809064: derivation stays proposed (gssreplay row for the Windows KMD's request words).
4. The lease-1 bare-suite log with a rental GPU UUID was pushed once before redaction (commit history).

## 7. Spend

Lease 1 56909569: 4.847 USD (destroyed 19:09 UTC). Lease 2 56939139: 1.299 USD (destroyed 20:36 UTC,
API-absent verified, timer cancelled after). Total 6.146 USD of the 12 USD cap. No lease is running.
