# Independent review of prod-candidate-6d (merge readiness)

**STATUS: ANSWERED, 2026-10-23. Read-only review of `origin/integration/prod-candidate-6d-20261023`
at `01a171e63` (draft PR #99) against `origin/master` `e9c6d39e5`. No product code, candidate or
lane branch was changed. No GPU, VM or guest was used; GPU-free tests ran on the host build farm.**

Scope: what is new since the candidate-6 review (`docs/audits/2026-10-22-candidate6-review.md` on
`claude/review-cand6-20261022`, which covered #76, #73 and #74 at `f7cf3ed81`):

1. #90, CE class seeded at birth (`5d2bc402b..9490d7227`) plus F1's compile fix
   (`3c07f8a33`, `01a171e63`).
2. #92, mirrored OLUT behind a signed matrix (`cb6e0587c..e636236cf`, merged at `70da04ffa`).
3. The combination of these with #73 (SA-02 privilege stamps).

Tags: **CODE READ** = read in the source at `01a171e63`. **MEASURED** = a command whose output I saw
for this review. **REPORTED** = measured by another lane; I read its evidence file but not its raw
logs. **INFERRED** = reasoning that a test or a run still has to confirm.

## Merge verdict

**MERGE AFTER FIXES.** No High finding. Nothing found lets a hostile guest widen anything: the CE
seed narrows what a Translated copy ring accepts, and the colour change adds no read and no index.
The fixes needed before merge are small and test-first (M1, M2, M3). The rest are follow-ups.

Required before merge:

1. **M1**: a GPU-free test that reaches the real `ChanPlane` birth wiring (host class, not guest
   class). Today no test fails if that wiring reads the guest's EngineObject class.
2. **M2**: make the seeded path visible (birth line plus counter), so a hardware run can show that
   the #90 path ran at all. Q4's Energy runs did not reach it.
3. **M3**: `kf-disp` `lockstep_threads_silence_and_return` failed 7 of 30 runs on the candidate and
   0 of 30 on master on the build farm (MEASURED 2026-10-23, p about 0.005). Its code path is
   unchanged (CODE READ). Before merge, decide whether it is a harness crossing race or a relay
   bug, and fix it. Otherwise master's CI goes red about 1 run in 4.
4. **L4** (stale text in `kf-qemu/src/display.rs` and one test doc): trivial, fold into the same
   PR.

Minimum hardware evidence still required (n >= 1 each, revision-stamped):

* **Cursor baseline (owner hold from Q4).** The same broker6 cursor RGB oracle on master
  `e9c6d39e5`, same box, same harness. If master also fails 48/52 with the extra-alpha fit, the
  failure is older than 6d and does not block this merge. My judgement (below): #92 is not the
  source.
* **#90 on hardware.** One run in which a dedicated-copy kernel ring submits work without a
  `SET_OBJECT` (seed counter > 0 after M2), with 0 `NoCeObject` and 0 `ForeignClass` refusals. The
  historical Energy COPY2 segment is the ideal case, but any workload that reaches it counts.
* **#92 on Windows.** A manual Hue / Digital Vibrance change on the 580.88 desktop with the console
  still updating (no `scanout REFUSED`, frame counter rising). The odd-vs-even readback on a real
  display engine (`V3_SDR_COLOR.md` falsifier) is research. It does not block the merge, because the
  alternative today is a frozen console.
* SA-02 chosen-root negatives: Q4 reports the guest RM returned different handles, so the forgery
  premise was not reachable on 580.88. The GPU-free coverage (`audit_kernel_channel_stamp.rs`,
  `copy_birth_policy.rs`) is enough for merge. Record that the premise was not reachable, not that
  it passed.

## Findings

File:line refer to `01a171e63`. Each finding has a section below with the scenario and a fix.

| # | Sev | Where | One line |
|---|---|---|---|
| M1 | Med | `kf-qemu/tests/copy_birth_policy.rs:132` | guest-class test cannot fail on the wiring |
| M2 | Med | `kf-qemu/src/chan.rs:7288` | seeded tier invisible; #90 fix unprovable |
| M3 | Med, blocks | `kf-disp/tests/lockstep_threads.rs:485` | candidate-only red: 7/30 vs 0/30 |
| L1 | Low | `kf-chan/tests/gr_tier_harness_birth_config.rs` | source-scraping rustc probe |
| L2 | Low | `kf-chan/src/tmode.rs:835-841` | pin refuses an older valid CE class |
| L3 | Low, old | `kf-chan/src/tmode.rs:893-898` | GR ring subch 4 still guest-chosen tier |
| L4 | Low, doc | `kf-qemu/src/display.rs:115-132` | stale "exact" / "no negative input" text |
| L5 | Low, evid. | `cuda/display/kf_color.cu:209-219` | odd extension is INFERRED |
| L6 | Low, style | `kf-cuda/src/display.rs` tests | whitespace-stripped source-text pins |
| C | carried | prior F1-F7 | all seven still open, none worse |

### M1: the wiring test cannot fail on the wiring

CODE READ. `guest_object(class)` sends a real `RmAlloc` for a guest CE object and checks that
`ChanStatement::EngineObject` is emitted. It then builds the ring with
`birth_decoder_config(a.engine_type.unwrap(), 0, 0xc7b5, false)`, where `0xc7b5` is a test literal.
The guest's class has no route to the ring under test. So the tests kill mutant M3 of
`mutants.json` (a constant injected *inside* the pure selector), but not the realistic regression:
`ChanPlane` passing the guest's EngineObject class (or `engine`-derived guesswork) instead of
`host.ce_class()`. The lane README states this boundary honestly ("does not ... execute
ChanPlane's host allocation path"). `ce_birth_20261018/README.md` names the security property as
"never a guest EngineObject class", and today it rests on CODE READ alone.

Concrete scenario: a later refactor changes `chan.rs:7194-7199` to the class of the first
EngineObject on the channel (which "looks" more faithful). Every GPU-free test stays green, and a
guest kernel can again pick the tier, e.g. `C8B5`-wide `upper_mask` on an Ada host.

Fix, test first: extract the four lines at `chan.rs:7189-7201` into a pure
`fn translated_decoder_config(engine, host: &impl HostRingFacts, gr_tier) -> GrConfig`, where
`HostRingFacts` exposes `ce_class()` and `video_context()`, with `HostRing` the only production
impl. Write the EXPECTED RED test with a fake whose `ce_class()` is `0xc7b5`, a guest EngineObject
of `0xc8b5` in the same plane, and assert `Tier::C7b5`. Then kill the mutant "use the EngineObject
class".

### M2: the seeded path is invisible on hardware

REPORTED (Q4 `energy-coverage.json`): 0/4 Energy failures, but the historical COPY2 channel
submitted and forwarded 0 entries in every trial. Q4's smoke run found 0 `NoCeObject` and 0 ring
refusals. That shows no regression. It does not show the fix. CODE READ: the BORN Translated line
prints `gr_tier`, `sw_subch_inert` and more, but not `gr.ce`. No counter records a launch decoded on
a seeded tier before any explicit bind. So even a run that *did* reach the path could not prove it.

Fix, test first: (a) add `ce_seed={:#x}` to the BORN Translated line. (b) Add a `TState` counter,
`seeded_launches`, incremented when a CE method is decoded while `tier` came from the seed and no
explicit `SET_OBJECT` of the host class was seen. Report it with the existing retire stats. Tests in
`copy_ring_birth_seed.rs`: the counter is 1 after `a_copy_runlist_kernel_ring_admits_...` and 0
after `the_own_host_class_can_still_be_bound_explicitly`. Then any Windows or Linux run gives the
hardware evidence for free.

### M3: a candidate-only failure in the lockstep harness

MEASURED (host build farm, `bf claude-review <branch> cargo test -p kf-disp --release --test
lockstep_threads`, 2026-10-23):

| Arm | Runs | FAILED | Order |
|---|---:|---:|---|
| candidate `01a171e63` | 30 | 7 | 4 alone, 16 alternating (candidate first), 10 reversed |
| master `e9c6d39e5` | 30 | 0 | 4 alone, 16 alternating, 10 reversed |

Every failure is the same test and the same violation: `lockstep_threads_silence_and_return`,
`"a frame without a credit (COMMIT.seq 0) 0 ms after the last tick"`. The other six tests in the
binary passed every time. One-sided Fisher p is about 0.005, so this is not noise. Q4's CI green
(2/2) fits a rate of about 23% (0.77^2, about 0.6).

CODE READ, by elimination: the test drives `kf-disp`'s engine, the real relay in `kf-broker` and
`kf-linux-raw`. In that source closure the candidate differs from master only in #92's
`kf-disp/src/color.rs` and `color/model.rs` (no manifest or `Cargo.lock` change), and the test calls
neither. INFERRED: #92 changes codegen or layout and so exposes a latent race at the
Silent-to-Steady transition (`scn.timeline` at 500 ms and 1100 ms). There are two candidates.
Either the harness check races a frame that the relay sent before it received the resumed tick
(the check uses the broker's *send* time, `lockstep_threads.rs:485-492`). Or the relay
(`kf-broker/src/conn.rs` `note_tick_live`) really sends an uncredited frame after going live. Only
the second is a product defect.

Scenario if merged as is: master's `cargo test --workspace` goes red on about 1 run in 4, and
"opportunistic CI" then hides real breakage, as it hid the E0063 in L1.

Fix, test first: (1) in the harness, record with each COMMIT whether the relay had already counted
the resume (`silence_resumes` at send time, or the frame's own credit state). Fail only an
uncredited frame sent *after* the relay went live. Run 50x per arm. (2) If the violation persists,
it is a relay bug: fix it in `conn.rs` behind a deterministic test in `engine/lockstep_soak.rs` that
crosses a resumed tick with a fallback frame. (3) If it is harness-only, fix the check and merge.

### L1: the compile-fix test and the gap that needed it

CODE READ. #90 added a field to `pub struct GrConfig`, but the harness binary (`kf-harness`, a
workspace member) still used struct literals. The CE lane's "complete suites" were
`-p kf-chan -p kf-qemu -p kf-rm` (`ce_birth_20261018/README.md`), so the E0063 reached the merge
commit `70da04ffa`. The fix is right (MEASURED: `cargo build -p kf-harness --release --bins`
finished at `01a171e63`, and the six probes passed). The guard, though, is a test in `kf-chan` that
`include_str!`s `kf-harness` source, cuts it at `.set_gr(` / `);`, and compiles it with `rustc` from
`PATH`, so it can differ from Cargo's toolchain. It breaks on a reformat and duplicates CI's
`cargo build --workspace`. Fix: make `kf-gr-tier` call `kf_chan::tmode::birth_decoder_config` with
its own `host*.ce_class()` (then GRAPHICS gets `ce = 0` by construction) and delete the probe. Lane
rule: a change to a `pub` type runs `cargo check --workspace --all-targets` before it claims green.

### L2: the class pin and older guest classes

CODE READ. `host()` refuses `SET_OBJECT` on subchannel 0-4, and a CE class on 5-7, unless the class
equals `gr.ce`, the newest class in *family set ∩ host list* (`kf-chip/src/host_classes.rs`). Before
#90 an explicit bind of any known CE class set the tier and authored `SET_OBJECT(class)` on the host
channel. That bind may well have failed on the host object, so the new refusal is the safer
behaviour. It is still a behaviour change across "all families are first-class": REPORTED evidence
covers Ada only (Q4 Linux 30/30, Windows desktop, 0 ring refusals). Fix, test first: a table test
over every family row that the CE class each guest stack binds (UVM per-arch class, Windows
twin) equals the derived host class. Or, if a family has two classes (Blackwell `C9B5`/`CAB5`),
decide by owner ruling whether a lower class maps onto the host object.

### L3: GR ring CE subchannel (pre-existing)

CODE READ. The pin applies only when `gr.ce != 0`, and `birth_decoder_config` sets it only for COPYn
engines. A kernel GR ring still binds any decoder-known CE class on subchannel 4 and picks the tier
(`wide_sem_addr`, `upper_mask`). `bind` still checks every address against the rows, so I found no
escape. Follow-up: pin subchannel 4 of a GR ring to the host GR channel's own CE object class.

### L4: stale text after #92

CODE READ. `kf-qemu/src/display.rs:115-121` still says each flag's change is "exact", and the
start-up line at `:132` still prints "MIRROR accepted on DIRECT ILUT/OLUT where no negative input
can reach it". Both stopped being true when the signed-matrix refusal went. The hue/vibrance test in
`kf-disp/src/color.rs` says "in every family row that has the C77D core", but it loops over two
driver versions (`580.65.06`, `580.159.04`). Fix: reword all three, with no behaviour change.

### L5: the odd extension is a reading, not a fact

CODE READ. `output_lookup` (`kf_color.cu:209-219`) and `model::value` implement
`lut(-x) = -lut(x)`. `V3_SDR_COLOR.md` marks it `[inferred]`, explains why odd and not even, and
states a falsifier (signed OCSC0, mirrored OLUT whose first sample is 0.25, scanout readback on bare
metal or VFIO). That is the right discipline. Until that readback exists, Hue/Vibrance colours on a
negative channel are a guess. The guess is visible and recoverable, unlike the freeze it replaces.

### L6: source-text pins

CODE READ. `the_output_kernel_packs_bgra_bytes` and
`the_output_launch_passes_the_tables_mirror_flag_last` strip whitespace from `kf_color.cu` and
`display.rs` and search for literal code. They pass on any behaviour that keeps the spelling, and
they fail on a harmless rename. Since the kernel runs only on a GPU, they are acceptable as drift
alarms. The real guards are the PTX parameter-type pin and the CL1 hardware run. Do not count them
as behaviour tests in mutation tallies.

## The combination (#73 x #90 x #92)

* **Reachability of the seed.** CODE READ: the seed exists only on a Translated birth. After #73 a
  Translated birth needs RM's `PRIVILEGE=KERNEL` stamp, so guest userspace never reaches it. Its
  COPYn channels are Passthrough and run natively. `copy_birth_policy.rs` checks that Windows C1E
  USER (0) and ADMIN (1) stamps stay Passthrough and that the census COPY2 birth (`0x16`) stays
  Translated. MEASURED at `01a171e63`: 5/5 pass.
* **A hostile guest kernel or root that forges the stamp** gets a Translated copy ring whose tier is
  the host's. Before #90 it could choose the tier with `SET_OBJECT`. The seed is therefore a
  narrowing. `engine` is guest-declared (`a.engine_type`), but the host ring for every non-GR/video
  Translated birth is on `me.host_ce` whatever the guest says (`chan.rs:7153`), so claiming COPYn
  only selects the host's own class.
* **Order and state.** `set_gr` is called once, at birth, before the first `next()`. No path resets
  `TState.tier` (grep: no `tier = None`, no `TState` rebuild). The shadow decoder runs only on non
  T-mode rings, so it cannot raise spurious `NoCeObject`. An unknown host class seeds `None`, which
  is the old behaviour (`NoCeObject` / `ForeignClass`).
* **#92 vs the other PRs.** It is disjoint: display only, and no shared state with the channel
  plane.

## #92: the colour kernel under a hostile guest

* **Bounds.** CODE READ: `output_lookup` adds no memory access. It calls the existing `lookup` with
  `-v`. `lookup` clamps with `sat()`, bounds `i <= entries-2` and refuses `entries` other than
  257/1025, so the highest read is `lut[(entries+3)*4+2]`. That is inside the
  `LUT_BYTES = 1029*8` snapshot, unchanged from master. `-inf` saturates to the last sample, and NaN
  is not `< 0` and samples entry 0. The matrix is S5.14 (21 bits), so `v` stays finite. The
  host-side extent checks in `color_output` are unchanged.
* **Result range.** The table is UNORM16, so `-lookup(..) <= 0` and the final `sat` makes it 0.
  Mirror therefore differs from master only where the pre-OLUT value is negative *and* sample 0 is
  not black.
* **Launch ABI.** The PTX diff is confined to `kf_color_output`. `kf_color_compose`,
  `kf_color_validate` and `kf_tmo_validate` are byte-identical apart from the header hash. The
  parameter list gained one `.u32`, and `display.rs` pins it. There is one launch site
  (`color_output`).
* **Caching.** `ColorDmas::resolve` already keys on the whole `Lut`, including `mirror`, so a toggle
  is a new token, never a stale binding. Both directions are tested.
* **Refusal log.** `ReasonLog` is bounded (64 reasons x 160 chars, linear `contains`), runs on the
  scan thread (not a vCPU, no new lock), and the counter still counts every refusal. Tested.
* **Removed refusal.** "Mirrored OLUT behind a signed matrix" is now rendered. That is a deliberate
  behaviour change (run 4500 froze at 0 fps), and its only cost is colour fidelity if the odd
  reading is wrong. `KF3_DISPLAY_LUT_MIRROR` stays default-off and is in the Windows prod flag set.
  This is a pre-existing flag that boot needs; the owner's no-flags rule still applies to it.
* **Kernel vs oracle.** REPORTED (`codex/color-validate-20261022`,
  `traces/color_validate_20261018/round2`, GTX 1660 SUPER, `e636236cf`): all four `color_mirror`
  cases match the hand-computed values `[0,160,64]`, `[64,160,64]`, `[192,0,0]` and `[192,64,64]`.
  The `color_sdr` and `color_tmo` examples show 16/16 checks unchanged. That evidence lives on the
  lane branch, not in the candidate. The GPU-free tests check the CPU model
  (`kf_disp::color::model`), not the kernel.

### Cursor RGB oracle 48/52: is #92 the source?

**Unlikely. My judgement is that it is not a 6d regression.** CODE READ:

1. The cursor is composed in `cuda/display/kf_bl_pack.cu` (VMM grab path) or sent to the broker as
   premultiplied ARGB (`kf-broker/src/cursor.rs`, `console.rs`). Neither file is in the candidate
   diff, and `kf_color.cu` has no cursor code.
2. On the Linux guest (Cinnamon, broker6) NVKMS never sets MIRROR. With MIRROR clear,
   `output_lookup` is exactly `lookup`, and the PTX of every other kernel is unchanged.
3. #90, #73, #74 and #76 touch no display code.
4. The fit is "RGB = reference x alpha / 255 within 1", which is a second premultiplication. That
   points at a straight/premultiplied convention mismatch in the cursor source or broker path (the
   broker contract is "premultiplied ARGB only"), not at an output LUT, which acts per channel
   without alpha.

INFERRED: pre-existing on master. This is what the master-baseline run above has to confirm.

## What holds up

* MEASURED at `01a171e63` (host build farm, 2026-10-23): `kf-chan` 676 passed / 0 failed /
  4 ignored, including `copy_ring_birth_seed` 15/15 and `gr_tier_harness_birth_config` 6/6.
  `kf-qemu --test copy_birth_policy` passed 5/5, and `kf-qemu --lib display::` 55/55. `kf-cuda`
  passed 76/76 GPU-free. `kf-disp` passed 237 lib tests (including the new hue/vibrance test) plus
  the integration tests, except M3. `kf-harness --bins` builds.
* MEASURED: `rustfmt --edition 2024 --check` on every changed `.rs` file in the candidate is clean.
* CODE READ: no new `unsafe` and no new `#[ignore]`. No `cfg(test)` branch in product code (the two
  new `cfg(test)` are model fixtures and tests). No test assertion was loosened by #90 or #92. The
  only removed assertions are #76's, and the prior review covered them.
* CODE READ: #73, #74 and #76 product code is unchanged since `f7cf3ed81`, apart from rustfmt and
  one comment re-attribution in `chanlink.rs:1411`. The prior review stands.
* CODE READ: the CE pin, the seed and `birth_decoder_config` are pure, allocation-free and O(1) per
  method. No new blocking, lock or unbounded loop, and nothing on a vCPU. No completion is forged:
  refusals kill the guest's own channel by name, as before.
* REPORTED (Q4, `01a171e63`): CI green on both hosted runs. Native gates 9/9 plus gate 10. Linux
  fast 30/30. Windows 580.88 desktop holds over 600 s, twice. App matrix 6/6. CATIA/Creo n=2 with no
  TDR. 0 `NoCeObject`, Budget or ring refusals in the smoke run.

## Carried over from the candidate-6 review (unchanged, still open)

CODE READ at `01a171e63`. None of them is a regression against master, and none blocks this merge.

* F1 `kf-qemu/src/mem.rs:1717-1725` and `twin.rs:57-61`: the mirror's initial KERNEL class still
  comes from `is_rm_internal_client`, and the docs still say "a handle range no guest process can
  hold".
* F2 `kf-mem/src/vasmgr.rs:1766`: a root move between chunks fails the invalidate and leaves it
  armed (`walk_busy`).
* F3/F4 `vasmgr.rs`: stale `(slot, root)` across chunks, and no `pump` after a failed
  continuation submit.
* F5 `kf-chan/tests/user_turn_unit.rs:1083-1087`: the transcript test still prints instead of
  asserting.
* F6 `kf-mem/tests/secaudit_all_pdb_over_64_spaces.rs:79`: the disjunctive assert is still there.
* F7 `docs/design/V3_USER_DESCRIPTOR_DECISION_20261022.md:3-6`: the STATUS still says "Nothing here
  is implemented".
