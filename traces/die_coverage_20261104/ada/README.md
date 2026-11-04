# Die coverage, Ada (AD10x), on the integration revision, 2026-11-04

STATUS: LIVE, 2026-11-04. Pre-registration first (written and pushed before any result was read); results are
appended under RESULTS. MEASURED = a line in a committed file in this directory, named with its file; INFERRED =
reasoning.

## What is booted

Revision under test: `0c515b3b3` = `origin/claude/integration-20261104` (next + the Windows gate +
Turing/Blackwell/Ampere fixes + caps rows + walk wake-up fix). Every box clones that branch
(`provision_full.sh`, then `merge_check.sh claude/integration-20261104 <tag>`); the kf3 binary is the one
`build_kf3.sh` installs under `kf3-bins/0c515b3b/`. Windows stage: `KF_MERGE_WINDOWS=1` (the gate as committed,
`scripts/bench/windows/kf3_flags.sample.env` unchanged). Host drivers: 595.91.07 on the Windows dies (AD102, AD104;
the earlier AD102 PASS g4 ran on 595.91.07), provisioning's default 580.159.04 on the Linux-only dies.

COMPUTED (`crates/kf-disp/data/caps_measured.tsv` at `0c515b3b3`): Ada (arch 0x190) has T3 `0x00c` rows for impl
0x2 (AD102), 0x3 (AD103), 0x4 (AD104), 0x6 (AD106) at branch 580 and impl 0x7 (AD107) at branch 595.

## Hypotheses and falsifiers (pre-registered)

- **H-linux (each die)**: `V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS|FALLBACK`, `KF3_RC=0`,
  `FAST_SUITE_PASS=30 ... ARMS=30`, `BIRTH_CENSUS_OK`. FALSIFIER: any of these missing or different.
- **H-win (AD102, AD104 if run)**: `WINDOWS_GATE_PASS ... resets=0` with `caps page derived ... (0 unmeasured)`.
  FALSIFIER: `WINDOWS_GATE_FAIL`, any reset counter > 0, or an `unmeasured` count > 0. RETRY is no verdict: re-run
  on a fresh box at most once.
- **H-gss595 (AD104 on host 595.91.07, own GPU only)**: the realize-time refusal `0x2080852a: body is 1716 bytes,
  its layout is 1784` is kayfabe's own size check on the 570-branch row (`kf_abi::gssreplay`, size 1716), not a
  host answer. Measured with the repo's kit (`kf-ctl-probe`, `KF_PROBE_GSS_ROWS=1` and `KF_PROBE_SCAN`, zero
  requests, unprivileged): the host accepts 852a at 1784 and refuses 1716. FALSIFIER: the host answers NV_OK to a
  zero 1716-byte 852a request (then the HOST_CONTROLS row is incomplete), or refuses 1784.
  For `0x2080852b`, `0x2080852c` and `0x20809064`: what the guest sends and what kf3 answers is read from this
  lane's own kf3 logs; the accepted zero-request sizes of 852b/852c are scanned with the same kit.
- **H-perf (AD104 box, same-box A/B, numbers only, no tuning)**: the extra ~60-100 ms per no-walk arm and the
  `uvm-invalidate`/`uvm-mean` slowdown (1.6-1.9x) seen on the Ada PC (`traces/walk_wake_20261103/`) are still
  present at `0c515b3b3`. Measured by running pre-#239 `9297ffdd9`'s kf3 with the same harness, fast guest and box
  (QEMU_BIN swapped, as `traces/ada_pc_20261103/tools/ada_pre_ab.sh` did). "Still present" iff the median no-walk
  arm is >= 50 ms slower than pre AND uvm-invalidate+uvm-mean is >= 1.4x pre; "gone" iff the median no-walk arm
  difference is < 20 ms AND the uvm ratio < 1.15; anything between is reported as unclear. Run only if budget and
  time allow.

## Boxes (own ids only)

| die | SKU | id | offer | where | created |
|---|---|---|---|---|---|
| AD102 | RTX 4090 | 59031339 | 58239659 | Spain | 2026-11-04T00:08Z |
| AD104 | RTX 4070 SUPER | 59031342 | 27266645 | Nevada US | 2026-11-04T00:08Z |
| AD103 | RTX 4080 SUPER | 59031595 | 52024799 | Denmark | 2026-11-04T00:11Z |
| AD106 | RTX 4060 Ti | 59031599 | 49744626 | Quebec CA | 2026-11-04T00:11Z |
| AD107 | RTX 4060 | - | - | no `vms_enabled` offer at 2026-11-04T00:11Z | - |

## RESULTS

Every line below is MEASURED on 2026-11-04 at revision `0c515b3b3` unless another revision is named, in the file
named with it. Host Xids: each merge check left 10 host Xid 31 lines (`ad10x/ev_*_xid.txt`), the suites' intentional
CE/PBDMA fault controls (5 from the bare suite, 5 from the fast suite; the shapes of `traces/walk_wake_20261103/`).

### Per die

| die | SKU, PCI id | box (Vast id) | host driver (found -> run) | v3 gates | fast suite (180 s) | census | Windows gate |
|---|---|---|---|---|---|---|---|
| AD102 | RTX 4090, `10de:2684` | 59031339, 30 vCPU, 48 GB, 155 GB | 575.51.03 -> 595.91.07 | 9/9 gate10=PASS | 30/30 (bare 30/30) | OK | **PASS x2** (ada102a, ada102b), resets=0, `known_live_dumps_193=0` |
| AD103 | RTX 4080 SUPER, `10de:2702` | 59031595, 24 vCPU, 39 GB, 117 GB | 575.51.03 -> 580.159.04 | 9/9 gate10=PASS | 30/30 (bare 30/30) | OK | not run (Linux-only die) |
| AD104 | RTX 4070 SUPER, `10de:2783` | 59031342, 17 vCPU, 24 GB, 155 GB | 575.51.03 -> 595.91.07 | 9/9 gate10=PASS | 30/30 (bare 30/30) | OK | **PASS** (ada104w), resets=0, `known_live_dumps_193=3` |
| AD106 | RTX 4060 Ti, `10de:2803` | 59031599, 15 vCPU, 49 GB, 117 GB | 575.51.03 -> 580.159.04 | 9/9 gate10=PASS | 30/30 (bare 30/30) | OK | not run (Linux-only die) |
| AD107 | RTX 4060 | - | - | - | - | - | **no `vms_enabled` offer** at 2026-11-04T00:11Z (searched with disk>=80, RAM>=12) |

Files: `ad10x/ada10xa.log` (merge-check summary: `V3_GATES_SUMMARY`, `KF3_BUILT .../kf3-bins/0c515b3b/`, `BARE_SUITE_PASS`,
`FAST_SUITE_PASS`, `BIRTH_CENSUS_OK arms=30 suite_births=161 gate_births=12`), `ada10xa_suite.out`, `ada10xa_gates.log`,
`ada10xa_births.log`, `prov.log`. H-linux not falsified on any of the four dies.

- **AD103 provisioning flake (rental/box, not the guest):** the first `provision_full.sh` ended `NOT_READY ... FG_RC=1`:
  `build_fast_guest: /dev/nbd0p1 never appeared after 5s` (`ad103/provision_fg_failed.log`); the image passed
  `qemu-img check`. `merge_check.sh` rebuilds the fast guest itself and got `FG_RC=0` on the same box minutes later; the
  bar above is that merge check. INFERRED: a slow udev partition scan on that box. Not fixed here (the 5 s wait in
  `build_fast_guest.sh` is the candidate if it recurs).

### AD102 Windows gate (H-win not falsified, n=2, one box, host 595.91.07)

`ad102/ada102a_windows.log`, `ad102/ada102b_windows.log`:

```
WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 4090" idle_s=300 hostref=green/green secs=2359 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=0 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-04T00:55:24Z
WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 4090" idle_s=300 hostref=green/green secs=906 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=0 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-04T01:10:59Z
```

`caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)`; head 0 lit 1920x1080 with a window
PUT (`+88288 ms the console shows head 0 1920x1080: [window 0 ...]`); 0 `USER_RING_SCAN`; `running_to_suspending=1`,
`host_xid_in_boots=0` (`win_a/.../gsp_signals.txt`, `win_b/gsp_signals.txt`); every reset counter 0 including
LiveKernelEvent 193 (`tdr_counts.txt`), where the earlier AD102 PASS g4 (`a5071c24`) had two 193 reports. Install 1436 s;
the second run on a fresh overlay of the same image 906 s.

### AD104 Windows gate (H-win not falsified, n=1, host 595.91.07)

`ad104/ada104w_windows.log` (run by hand from the merge check's checkout after the A/B, fresh install on this box):

```
WINDOWS_GATE_PASS rev=0c515b3b gpu="NVIDIA GeForce RTX 4070 SUPER" idle_s=300 hostref=green/green secs=5065 install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 known_live_dumps_193=3 stale_lke_resubmits=0 strict=0 enable_utc=2026-11-04T02:40:13Z
```

`caps page derived — 56 words: 55 T1 rule, 0 T2 host, 1 T3 measured (0 unmeasured)`, head 0 lit with a window PUT,
`running_to_suspending=1`, `host_xid_in_boots=0`, every reset counter 0, LiveKernelEvent 193 = 3 (reported, not gated,
as on AD102 g4, TU106 and GA102) (`ad104/win_w/`). Install 2918 s, `nv_install` 728 s on this 17-vCPU box. The same GSS
pattern as AD102 (`ad104/win_w/gss_lines.txt`): 9064 135 of 136 calls refused (`unmeasured on AD104 branch 580`; the
AD104 row in the table is branch 595), 852c 6 refused, 852a 4 served; 852b not asked on this die.

### The GSS controls on host 595.91.07 (own GPU only; H-gss595 not falsified)

Measured with the repo's kit, `kf-ctl-probe` built from `0c515b3b3`, run as an unprivileged user (`Uid 1002`, `CapEff 0`)
on our AD104 at 595.91.07, 00:54Z, host Xid count 10 before and 10 after (`ad104/gss595/tier2_probe_trimmed.txt`,
`scan_852abc_9064_trimmed.txt`; zero requests, sizes 4..16384 step 4):

| control | sizes the host accepts (zero request) | what the Windows 580.88 guest sends (AD102, `W343GSSCENSUS`) | at `0c515b3b3` |
|---|---|---|---|
| `0x2080852a` | every size from 1784 up; 1716 -> INVALID_ARGUMENT | 1784 | served from the host's realize-time reply (4 calls) |
| `0x2080852b` | only 3324 | 2836, words 0..9 `{0,0,0x1f,0,...}` | refused (3-4 calls) |
| `0x2080852c` | only 1440 | 1440, words 0..9 `{0,0,0x1f,0,...}` | refused (6 calls) |
| `0x20809064` | none (a zero request is never accepted); cudart's and the encoder's authored requests NV_OK at 520 | 520, words 0..9 `{0,3,1,0,0,0,4,0,0,0}` | 137-138 of 138-139 calls refused (`W349REFUSE ... consttab cudart.0x20809064: unmeasured on AD102 branch 580`) |

- The startup line `HOST-ABI REFUSED control 0x2080852a: : body is 1716 bytes, its layout is 1784` is kayfabe's own
  size check on the 570-branch row (`kf_abi::gssreplay`, size 1716), raised before any ioctl, on 580.159.04 hosts too
  (AD103, AD106 `realize_gss_lines_timer_arm.txt`). The host refuses 1716 anyway, so no answer is lost. The cosmetic
  defect is the follow-up line `host facts: GSS 0x2080852a [] refused by the host (status: None ...)`, which says "by
  the host" for a question that never reached it. No change proposed.
- `0x2080852b`: the guest's 580-branch size (2836) is not a size this host accepts for a zero request, and the guest's
  own question (2836 B, word 2 = 0x1f) is refused too: `GSSROW cmd=0x2080852b size=2836 ... status=0x1f INVALID_ARGUMENT`
  for both probes (`ad102/gss595/gss_rows_probe_f08c861ea.txt`, kf-ctl-probe built from the tentative row `f08c861ea`,
  unprivileged, our AD102, 02:13Z, host Xid 10 -> 10). No fix is derivable at 595.91.07; the tentative row is reverted
  (`66beba5fe`). A 580.x host was not asked (the 580 boxes were gone by then): unmeasured.
- Not asked here, seen in the same probe: 595.91.07 answers `0x2080852e` from 596 up, `0x2080853a` from 15352 up,
  `0x2080a618` only at 8632, `0x2080853b` at 14320 with word 1 = 1, but kf3's host ABI refuses all four at that host
  (`NoHeader`/`UnlistedControl`, `realize_gss_lines_timer_arm.txt` of AD104). That matters only for a 595-branch guest;
  the 580.88 guest sends 528/1456/8620/1832, which this host refuses. Recorded, not changed.

**Proposed fix (on this branch, not in `0c515b3b3`; three patched gate boots on AD102, all `WINDOWS_GATE_PASS ... resets=0`):**

| run | revision | change | result |
|---|---|---|---|
| ada102p | `053d5cab5` | exact all-zero `0x2080852c` row at 1440 + `W343GSSREQ` (first ten request words of 9064) | **852c falsifier fired**: realize asked the row without a refusal, the guest's 6 calls stayed refused, so its request is not all-zero. 9064 request words recorded |
| ada102q | `ec9f8681c` | 852c zero row removed; **9064 row with the guest's own inputs** `{(0,0),(4,3),(8,1),(0x18,4)}`; 852c words logged | **9064 falsifier did not fire**: no realize refusal, the guest asked 9064 **once** and was answered (`result=0x0`, `GSS 0x20809064 answered from the host's realize-time reply`) where `0c515b3b3` refused 137 of 138 calls (ada102a; the commit message of `1c24a2562` says 52 of 53, a count read while that boot was still running). 852c words `{0,0,0x1f,0,...}` |
| ada102r | `12063d4f8` | 852c row with the guest's own inputs `{(0,0),(4,0),(8,0x1f)}`; 852b words logged | **852c falsifier did not fire**: no realize refusal, all 6 guest calls answered (`result=0x0`, `GSS 0x2080852c answered from the host's realize-time reply` x6); 9064 again 1 call, answered. 852b words `{0,0,0x1f,0,...}` at 2836, still refused |

**The proposal is `12063d4f8`** (its `crates/` tree equals the branch head `66beba5fe`): two `gssreplay` rows (9064 and
852c with the Windows guest's own request words) plus one host-ABI row (852c at 1440 B, 595.91.07), and the bounded
read-only `W343GSSREQ` diagnostic (which can be dropped before a merge). Each row is the host's own answer to the
guest's own question, asked once at realize with the zero and background probes (`kf_abi::gssreplay`); no captured
bytes. The same rows were asked by kf-ctl-probe on our AD102 at 595.91.07: 9064 `(4,3)` and 852c `(8,0x1f)` NV_OK for
both probes (`ad102/gss595/gss_rows_probe_f08c861ea.txt`). Not shown: the rows on a 580.x host or on any other die
(the 595 row of 852c is scoped to 595.91.07; 9064's host row already covers 595.91.07 and the 580 interval). Tests: `cargo test -p kf-abi -p kf-rm` green; the new
`gssreplay` tests were red before their rows. INFERRED, not measured: why the guest re-asks 9064 ~138 times when refused
and also ~129 times on the old tree that served the GA106 capture (g4), but once when given the host's answer.

### Perf: the extra cost of no-walk arms and the uvm arms (H-perf)

Same box (AD104, 17 vCPU nested), same harness and fast guest (the `0c515b3b3` checkout), `QEMU_BIN` swapped to
pre-#239 `9297ffdd9`'s kf3 (`kf3-bins/9297ffdd/`), two runs each, all four 30/30 (`ad104/ab/ab_summary.txt`, the per-arm
tables `ab_int*_vs_*.txt` made with the Ada lane's `ab_compare.py`):

- the 21 no-walk arms: integration minus pre, median **+131, +171, +259, +221 ms** over the four cross pairs (mean of two
  vs mean of two: +190 ms), against same-revision pairs of -159 ms (int vs int) and +11 ms (pre vs pre);
- `uvm-invalidate` + `uvm-mean`: integration/pre **1.07, 1.17, 0.99, 1.08** (1.08 mean of two);
- whole suite client time: integration/pre 0.92-0.95; per-walk latency about 125-140 us at `0c515b3b3` against about
  210-365 us at `9297ffdd9` on this box (vat columns of the same tables).

Against the pre-registered rule this is UNCLEAR (the no-walk condition holds, the uvm condition does not). In plain terms:
the extra per-arm cost of arms with no walks is still there on this revision (+130..+260 ms median on a nested box whose
arms take ~1.8-2.2 s); the 1.6-1.9x `uvm-invalidate`/`uvm-mean` slowdown of the Ada PC is not reproduced here
(<= 1.17x). Not tuned. INFERRED: the nested box's 10-40x exit cost may hide or inflate both effects; the PC remains the
place to bisect them.

## Not measured / open

- AD107 (RTX 4060): no rentable `vms_enabled` offer at 2026-11-04T00:11Z.
- AD103, AD106: Windows not run (Linux-only dies in this lane); AD104 and AD102 Windows on host 595.91.07 only.
- The GSS proposal (`12063d4f8`) is validated on AD102 only (three patched boots, all PASS); not on AD104, not on a 580.x
  host, not by the Linux suites (the Linux fast guest asks none of 852b/852c/9064-with-word-1=3; GPU-free tests only).
- `0x2080852b` on a 580.x host; the 595-branch-guest sizes of 852e/853a/a618/853b at the host ABI (recorded above).
- The cause of the no-walk-arm extra cost (A/B above): not bisected.

## Cost and teardown

| id | die | rate (Vast, USD/h) | lease (UTC) | cost (INFERRED from rate x time) |
|---|---|---|---|---|
| 59031339 | AD102 | 0.578 | 00:08:43 - 02:14:48 | ~1.21 |
| 59031342 | AD104 | 0.207 | 00:08:43 - 02:58:45 | ~0.59 |
| 59031595 | AD103 | ~0.28 (offer 0.269 before disk) | 00:11:38 - 00:49:26 | ~0.18 |
| 59031599 | AD106 | ~0.30 (offer 0.292 before disk) | 00:11:38 - 00:41:41 | ~0.15 |

Total about 2.1 USD (budget 3.5). All four destroyed by id (`vastai destroy instance <id> -y`); `vastai show instances-v1
--raw` at 02:58:45Z lists none of them (it lists two ids of other sessions, left alone). The account credit moved
35.86 -> 27.97 USD over the same window, which includes other lanes' leases.

Evidence hygiene: GPU UUIDs (host board and the gate's per-VM UUID) are replaced by `fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001`
at the branch head (`scripts/ci/no_gpu_uuids.py`: new=0); screenshots, dxdiag, setupapi and PnP dumps were not committed.
⚠ Two intermediate commits of this branch (`AD102 Windows gate PASS ... (run ada102a)` and the next evidence commits up
to `02baed140`) still carry the AD102 host's board UUID in `win_a/` files; collect by squash (or filter those paths) to
keep it out of `next`'s history. The same commits (from `40a439a7c`) also carried two desktop screenshots (2.4 MB) and the guest's dxdiag/setupapi/PnP dumps, re-extracted by a late copy after the trim; they are removed at the head.
