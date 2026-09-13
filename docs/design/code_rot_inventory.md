# Code-rot inventory — what is dead, what lies, and what it costs

> **STATUS: LIVE, 2026-09-13 (w651).** Read-only audit at `HEAD = f27a60eb`, goal 5 of the
> owner's 2026-09-13 standing directive (*"code rot cleaned or marked for mechanical cleanup"*).
> **Nothing in this audit was changed** — no `.rs`, no `Cargo.toml`, no existing doc. This file
> is the marking; the cleanup is a separate act.
>
> ⚠ **Line numbers were re-derived at `f27a60eb` immediately before writing.** Seven crates were
> being edited by other agents during the audit and `HEAD` moved once under it
> (`be54b720 → f27a60eb`). A citation that does not resolve should be re-grepped by name, not
> assumed wrong.
>
> ⊘ **This inventory supersedes nothing.** `ORPHANS_wire_or_discard.md` (LIVE, 2026-09-12) stays
> the decision ledger for the six isolate verbs and the wire tags; this file is the wider sweep
> and it **corrects one row of that ledger** — see R21.

---

## 0. How this was measured, and where the method fails

Three passes, in this order:

1. **Name-level reachability from the real entry points.** The production entry points are
   (a) the 23 `#[unsafe(no_mangle)] extern "C"` symbols in
   `crates/kayfabe-qemu-raw/src/shim_unsafe.rs`, which QEMU's C shim calls, and
   (b) `crates/kayfabe-isolate-host/src/bin/isolate.rs`. `rmladder` and `sandbox_probe` are
   probe binaries and are counted separately as `bin`, never as production.
2. **Direct re-verification of every candidate** with `grep -rn '\bNAME\b' crates tests tools`,
   classifying each hit as declaration / production / test-target / doc-comment-only.
3. **Cross-check against the committed w256 sweep** (`traces/gates/orphan_sweep_w256/triage_all.tsv`).

### ⊘⊘ The method's own false positives, measured — do not repeat them

★ **A grep for `COUNTER.fetch_add` is not a test for "is this counter written."** This tree
writes counters through **reference-passing helpers**, and three separate shapes defeat the
obvious grep:

| shape | site | what the naive grep reports |
|---|---|---|
| a `match` selecting `&STATIC`, then `counter.fetch_add(..)` | `crates/kayfabe-isolate-host/src/rm.rs:1467-1497` (`birth_census::tally`) | `GUEST_RING`, `DECLINED`, `NOT_ASKED` all read as **never written**. They are written. |
| a helper taking `&AtomicU64` | `crates/kayfabe-vmm-qemu/src/lib.rs:481` (`Audit::bump(&p.audit.live_windows, &p.audit.peak_windows, 1)`, called at `:1700`, `:2273`) | ~18 census fields read as **never written**. All are written. |
| reads through a closure `g(&self.field)` | `crates/kayfabe-vmm-qemu/src/lib.rs:499-512` | the same ~18 fields read as **never read**. All are read. |

⇒ **Every class-2 entry below was confirmed by reading the declaring module, not by the grep.**
The task brief prescribed "grep for each counter's mutation site" — on this tree that check
returns false positives at a rate of roughly 20:1, and reporting from it unfiltered would have
produced an inventory that was mostly wrong. ⚠ Same family as the tree's own
*"SUSPECT THE INSTRUMENT FIRST"*: two of my own candidates (`SetPageDirPolicy` "never seated",
`sweep_pt_tables_from` "the sweep is dead") were **refuted by the follow-up grep** and are
recorded below under *Checked and ruled out* so nobody re-files them.

---

## 1. The ranked table

Cost order: a row that can silently produce a **wrong number** or a **wrong routing decision**
outranks a row that is merely unused. `M` = MECHANICAL (safe without design input),
`D` = NEEDS-DECISION.

| # | class | what | where | cost if left | |
|---|---|---|---|---|---|
| **R1** | 5 dup | `RamRegionId`'s "address-derived" tag bit is hand-encoded at 5 sites and **one already uses bit 62 where four use bit 63** | `kayfabe-vmm-qemu/src/lib.rs:2054` vs `:1134`, `kayfabe-vmm-kvm/src/lib.rs:848,1225`, `kayfabe-mocks/src/lib.rs:1073` | **wrong routing, silently.** These ids key `BTreeMap`s with overwrite semantics; one harmonising edit collides two regions and a window's backing is clobbered with no error | **D** |
| **R2** | 3 doc | `DoorbellAsyncArm::Off` is documented as *"★ THE CONTROL … byte-identical to every boot before w383"*; `[measured w529]` the `off` boot **cannot initialise the adapter** | `kayfabe-qemu-raw/src/shim.rs:18674` (the claim) vs `:16044` (the measurement, 2 600 lines away) | **an A/B against a control that does not boot.** The correction exists in the file but not above the text it corrects — the house rule's exact failure shape | **D** |
| **R3** | 1+3 | `take_table_changes` has **zero callers**, while its sibling's doc calls it *"the publication trigger"*. The live trigger is a `last_table_epoch` swap | `kayfabe-rt/src/device.rs:4562` (dead) · `:4541` (the claim) · `kayfabe-qemu-raw/src/shim.rs:16390` (the real trigger) | an auditor asking *"what triggers publication?"* is pointed at code that never runs | **D** |
| **R4** | 5 dup | `AMPERE_COMPUTE_B = 0xc7c0` re-typed as a local `pub const` in the decoder that recognises the compute bind, instead of importing the generated one | `kayfabe-rt/src/completion_watch.rs:63` vs canonical `kayfabe-abi/src/generated/classes.rs:287` | regenerate the ABI table and this decoder keeps the stale id — `cuCtxCreate`'s compute bind stops being recognised, **no compiler error** | **M** |
| **R5** | 1 | The **(client, vaspace)-keyed** resolver triple is dead; the live path is PDB-keyed | dead: `kayfabe-device/src/plane.rs:3118`, `:3161`, `:3216` · live: `:3369` `published_root` + `:3404` `resolve_va_from_root` (`shim.rs:7541,11400,11415,11463`) | **the dead one is 250 lines earlier in the file and carries the ★★★ trigger-discipline doc.** A reader looking for "how to resolve a published VA" finds the wrongly-keyed one first — and `va_spaces_are_keyed_by_their_page_directory_base` is a live open bug | **D** |
| **R6** | 2 | `SetPageDirRefusal` — a refusal enum with **no producer anywhere**. Only the declaration, two `Self::` arms in its own tag impl, and an `assert_send_sync!` | `kayfabe-device/src/setpagedir.rs:315` (`Serialized` :321, `SizeMismatch` :328), tag arms `:341,:346`, `:498` | a refusal **vocabulary entry that cannot occur** — the inverse of the tree's *"refuse by name means the name is TRUE"* rule. A reader counts two refusal paths that do not exist | **D** |
| **R7** | 1+2 | `WalkResult` — self-described *"Placeholder shape for the milestone"*, **never constructed, never matched**, yet publicly re-exported from the MMU crate's root | decl `kayfabe-mmu/src/walker.rs:118` · re-export `kayfabe-mmu/src/lib.rs:1443` | a **public API type** of the MMU crate that answers nothing. The real answer is `Translation` (`walker.rs:133`), re-exported on the next line | **M** |
| **R8** | 5 dup | `KAYFABE_BUILD_REV` stamping is copy-pasted between two `build.rs`, and **the copy lacks the `.git` rerun triggers** | `kayfabe-qemu-raw/build.rs:24-28` (has `rerun-if-changed=../../.git/HEAD`, `../../.git/refs/heads`) vs `kayfabe-isolate-host/build.rs` `fn stamp_build_rev` (has neither) | **two binaries from one tree reporting different revisions.** CLAUDE.md: *"the bench silently served a binary built from `862c7c2` for weeks"* — this is the same defect, self-inflicted | **M** |
| **R9** | 3 doc | `[Spine::materialize_isolate]` — a rustdoc intra-doc link to **a function that does not exist**, plus a false *"the only caller"* claim | `kayfabe-core/src/gpu.rs:3307-3308`. `grep -rn 'materialize_isolate' crates` returns **only this comment**. The real fn is `materialize_pending` (`gpu.rs:5455`) with **three** call sites: `gpu.rs:5440`, `shim.rs:5384`, `shim.rs:16292` | a reader trusting it believes isolate spawning is vCPU-synchronous and single-sited. `shim.rs:5384` is labelled *"w479 — the isolate spawn, on the worker where it belongs"* — the comment predates that move. ⚠ **CI runs no `cargo doc`**, so broken intra-doc links are invisible (`.github/workflows/ci.yml` has `cargo build/clippy/fmt/test`, no `doc`) | **M** |
| **R10** | 1 meta | **The orphan ratchet was never installed, and 13 new zero-caller items have accrued since the baseline** | `traces/gates/w258_dead_set_proposal.md` §3.2 recommendations 2 and 3; `scripts/orphan_gate.sh` has no ratchet/baseline/exemption logic (`grep -n 'RATCHET\|exempt\|baseline'` → only a *"baseline: cargo check"* liveness probe); `grep -n orphan scripts/ci_gates.sh` → **0** | the gate ran **once**, at w256, ~400 commits ago, and is in no CI. R11–R24 below are what that bought | **D** |
| **R11** | 1 | `crates/kayfabe-device/src/gpgaview.rs` — **the whole module**, 14 public items, zero callers. Its only outside mention is a doc comment that cites it as authority | `kayfabe-device/src/lib.rs:63` (`pub mod gpgaview;`) is the sole code reference; `kayfabe-mmu/src/refresh.rs:137` cites `ViewSpace` as *"what … models"* the peer-memory axis | a **deferral justified by citing an empty module** — the `the_viewspace_citation_nobody_audited` memory entry, still standing | **D** |
| **R12** | 4 | `mute_doorbell_witness` is called **only by `rmladder`**; the shipping isolate never mutes, so every production boot prints **two `eprintln!` per doorbell** for the first 512, on the submission path | `kayfabe-isolate-host/src/rm.rs:372` (decl) · only caller `src/bin/rmladder.rs:9853` · the printer `rm.rs:1807-1830` | its own doc says the lines are *"a large fraction of the number"* a submission timer reads. Every latency figure taken from the **production** isolate carries the printer's cost | **D** |
| **R13** | 3 doc | `ORPHANS_wire_or_discard.md` (STATUS: LIVE, 2026-09-12) still says `KAYFABE_PENDING_VIA_LOCK` *"is already settled and should go"*. **It went.** | `docs/design/ORPHANS_wire_or_discard.md:38` vs `kayfabe-device/src/plane.rs:4289`: *"⊘ THE w467 A/B IS SETTLED AND ITS LOSING ARM IS GONE"* | the tree's own to-do list carries a completed item — `a_blocker_i_declared_was_already_fixed` | **M** |
| **R14** | 3 doc | `gpga_address_space.md` header says `Status: **designed, not built**` while **its own §8.4** says `★ BUILT (2026-07-30)` and cites types that exist | `docs/design/gpga_address_space.md:3` vs the same file ~`:264`, citing `kayfabe-mmu/src/lib.rs:77` (`HostSlice`), `:156` (`HostExtent`), `:197` (`HostBacking`) — all present | GPGA is a **governing owner directive**; its design doc reads as unbuilt | **M** |
| **R15** | 3 doc | **25 of 173 design docs carry no STATUS block at all** — they *"read as current forever"* by the tree's own rule | `RESUME_HERE_w494.md`, `THE_OVERNIGHT_DIRECTIVE.md`, `address_kinds.md`, `boot_measured_2026_08_01.md`, `c_bug_regression_matrix.md`, `c_cuda_ladder.md`, `compatibility_matrix.md`, `core_state_and_consolidation.md`, `four_axes_of_variation.md`, `fuzz_campaign_2026_08_01.md`, `gpu_compartmentalisation.md`, `gsp_boot_gate_spec.md`, `isolate_founding_rationale.md`, `l1_architecture_summary.md`, `l2_qemu_adapter.md`, `mock_fidelity_audit.md`, `no_non_gsp_boot_path.md`, `open_questions_for_the_owner.md`, `replay_audit.md`, `resume_from_fault.md`, `ring_write_path_map.md`, `three_invariants_audited_w430.md`, `translated_bus_aperture.md`, `vmm_portability_seam_audit.md`, `w288_error_delivery_gate.md` | census below | **M** |
| **R16** | 3 doc | **4 superseded docs carry no back-reference**, so a reader arriving at the stale one is never told | `RESUME_HERE_w392_overnight.md` (superseded by `RESUME_HERE_w394.md:3`) · `RESUME_HERE_w494.md` (by `RESUME_HERE_w530.md:3`) · `RESUME_HERE_w417.md:309` §w426 (by `PLAN_barrier_driven_refresh_w434.md:3`) · `fbwin.rs` module docs (claimed by `gpga_is_one_reserved_object.md:4`, see S3) | the house rule names this exact shape and prices it at two wasted bench lanes | **M** |
| **R17** | 2 | `SLOW_TRAP_SITE_LAST` — **stored, never loaded**. Its doc says it *"survives a larger trap elsewhere taking the global maximum"*, i.e. it exists to be reported. It is reported nowhere | `kayfabe-util/src/trapwitness.rs:219` (decl), `:1045` (the only `.store`). `grep -rn 'SLOW_TRAP_SITE_LAST' crates tests tools` returns **exactly those two lines** | an instrument that advertises a fact and never delivers it. Trap-census readers assume the site is available | **M** |
| **R18** | 1 | `StaticInfoPolicy::with_gid` — a public parameterised constructor whose **only caller is `new`, passing the derived value** | `kayfabe-device/src/staticinfo.rs:84` (decl) · `:78` (`StaticInfoPolicy::with_gid(chip, driver, Self::gid_for_chip(chip))`) | the GID reads as configurable and is not | **M** |
| **R19** | 1 | `sweep_pt_tables_from` — an orphaned **wrapper**. ⊘ The mechanism is **live** via `sweep_pt_tables_revoking` | dead wrapper `kayfabe-rt/src/device.rs:4350` (delegates at `:4356`) · live `:4368`, called from `kayfabe-qemu-raw/src/shim.rs:10949` | low cost, but the wrapper carries the ★★★★★ module-level doc for the whole sweep, so deleting it must move that text, not drop it | **D** |
| **R20** | 1 | `reassembler` — zero production callers; one test | `kayfabe-rmrpc/src/policy.rs:1187` · only `tests/tests/rmrpc_bridge.rs:8080` | already a WIRE row in `ORPHANS_wire_or_discard.md`; recorded here for completeness, **not** a new finding | **D** |
| **R21** | 1 | **20 further items with zero callers of any kind** (not the declaration, not a test, not a probe binary) | see §2 | individually cheap; collectively they are what R10 costs | **M** |
| **R22** | 2 | **11 enum variants that are never constructed**, across 8 crates | see §3 | each is a branch nothing takes; the refusal ones (R6, `ViewFault::BadRegion/BadSlice`, `InstallRefusal::Index/Vmm`, `FaultEmitRefusal::ChidOutOfRange`) are refusal vocabulary that cannot fire | **D** |
| **R23** | 5 dup | `RM_PAGE_SIZE = 4096` defined three times inside `kayfabe-device` alone, under two names | `src/abi.rs:64` · `src/ga10x.rs:583` (`ACCESS_COUNTER_RM_PAGE_SIZE`) · `src/ga10x.rs:1830` | feeds a notify-buffer entries-per-page computation (`ga10x.rs:1856`) and a region page-size field (`abi.rs:104`). The value is architecturally fixed, so this is hygiene, not a live bug | **M** |
| **R24** | 1 | `CTRL_NO_REPLY` — a `pub const` referenced **only from doc-comment text**, never from code | `kayfabe-qemu-raw/src/shim.rs:1372` (decl); the four other hits are `///` lines at `:1409,:1413,:1439,:1444` | the sentinel it names is written by literal elsewhere, so the const documents a convention nothing enforces | **M** |

---

## 2. R21 — the 20 verified zero-caller items

Every row: `grep -rn '\bNAME\b' crates tests tools --include='*.rs'` returns the declaration and
**nothing else** except doc-comment lines. The `doc-refs` column is how many `///` lines link to
it — a high number means the item is load-bearing *in the documentation* and absent from the code.

| item | site | doc-refs | in w256 baseline? |
|---|---|---|---|
| `fb_join_installed_at` | `kayfabe-device/src/plane.rs:2646` | 7 | new |
| `read_published_va` | `kayfabe-device/src/plane.rs:3161` | 8 | new |
| `resolve_published_va` | `kayfabe-device/src/plane.rs:3118` | 6 | new |
| `published_walk_trace` | `kayfabe-device/src/plane.rs:3216` | 1 | new |
| `set_page_dir_log` | `kayfabe-device/src/plane.rs:3074` | 0 | `NO_CALLER_ANYWHERE` |
| `bar0_window_write_count` | `kayfabe-device/src/plane.rs:3771` | 0 | new |
| `any_pending` | `kayfabe-device/src/cpuintr.rs:479` | 0 | `NO_CALLER_ANYWHERE` |
| `take_table_changes` | `kayfabe-rt/src/device.rs:4562` | 8 | new |
| `guest_leaf_census` | `kayfabe-rt/src/device.rs:5769` | 0 | new |
| `retired_fb_joins` | `kayfabe-rt/src/device.rs:5576` | 0 | new |
| `fb_frame_va_in_vas` | `kayfabe-rt/src/device.rs:5369` | 1 | new |
| `observe_ce_release_targets` | `kayfabe-rt/src/ceutils.rs:1292` | 3 | new |
| `census_gr_addresses` | `kayfabe-rt/src/ceutils.rs:1656` | 2 | `NO_CALLER_ANYWHERE` |
| `contains_set` | `kayfabe-util/src/coverage.rs:271` | 0 | new |
| `trap_entries` | `kayfabe-util/src/trapwitness.rs:245` | 0 | new |
| `point_name` | `kayfabe-mmu/src/blockage.rs:292` | 0 | new |
| `attribution_note` | `kayfabe-core/src/fault.rs:111` | 1 | `NO_CALLER_ANYWHERE` |
| `write_back_attainable` | `kayfabe-linux-raw/src/memtype.rs:276` | 0 | new |
| `clear_memslot` | `kayfabe-linux-raw/src/kvm_unsafe.rs:474` | 1 | `NO_CALLER_ANYWHERE` |
| `from_fd` | `kayfabe-linux-raw/src/chardev_unsafe.rs:157` | 0 | `NO_CALLER_ANYWHERE` |
| `declare_device_window` | `kayfabe-vmm-kvm/src/lib.rs:1221` | 0 | `NO_CALLER_ANYWHERE` |
| `sink_mut` / `emit_now` | `kayfabe-trace/src/sink.rs:225` / `:300` | 0 | `NO_CALLER_ANYWHERE` |
| `build_rev` | `kayfabe-qemu-raw/src/lib.rs:115` | 0 | `NO_CALLER_ANYWHERE` |
| `host_alloc_vaspace_space` | `kayfabe-isolate-host/src/rm.rs:4476` | 0 | new |

★ **13 of these are NEW since the w256 baseline** — they did not exist, or were not dead, when
the gate last ran. That is the accrual R10 is about, and it is the number that makes the case for
recommendation 2 of `w258_dead_set_proposal.md` (*"enforce `NO_CALLER_ANYWHERE` as a RATCHET off a
committed baseline"*).

⊘ **The 164-row w256 `NO_CALLER_ANYWHERE` set is NOT re-reported here, and must not be bulk
deleted.** `w258_dead_set_proposal.md` §3 already adjudicated it: 45 are the `kayfabe-linux-raw`
OS-portability seam, 26 are the `kayfabe-abi` mirror, 5 are `kayfabe-mocks` test doubles —
**for those three crates `NO_CALLER_ANYWHERE` is the designed state.** That analysis stands; what
is missing is only its recommendations 2 and 3.

⚠ **I could not soundly re-measure how many of the 164 survive.** A name-level recheck is
meaningless for the set's most common names (`is_empty` ×10, `len_bytes` ×6, `decode` ×6,
`new` ×5, `len` ×4) — they collide with hundreds of unrelated items. The only sound instrument is
`scripts/orphan_gate.sh` itself, re-run. **Do not quote a survival figure that was not produced
by the gate.**

---

## 3. R22 — enum variants never constructed

`grep -rn 'Enum::Variant' crates tests tools`, plus a read of the declaring file for bare and
`Self::`-qualified construction. Listed only where **no construction site of any kind** exists.

| variant | site | note |
|---|---|---|
| `SetPageDirRefusal::Serialized` | `kayfabe-device/src/setpagedir.rs:321` | R6 — refusal that cannot fire |
| `SetPageDirRefusal::SizeMismatch` | `kayfabe-device/src/setpagedir.rs:328` | R6 |
| `WalkResult::Mapped` / `::Fault` | `kayfabe-mmu/src/walker.rs:120` / `:125` | R7 — the whole enum |
| `ViewFault::BadRegion` | `kayfabe-mmu/src/gpga.rs:435` | siblings (`UnknownViewer`, `SelfAlias`, `ViewOffsetOverflows`) **are** constructed at `:726,:738,:745` — so this is a gap in a live vocabulary, not a dormant type |
| `ViewFault::BadSlice` | `kayfabe-mmu/src/gpga.rs:518` | as above |
| `InstallRefusal::Index` / `::Vmm` | `kayfabe-vmm-qemu/src/viewer_install.rs:393` / `:395` | refusals with no producer |
| `FaultEmitRefusal::ChidOutOfRange` | `kayfabe-rmrpc/src/fault.rs:52` | a chid bound that is never enforced by this name |
| `EventNotifyError::NotifierNotSilent` | `kayfabe-abi/src/eventnotify.rs:580` | |
| `EventNotifyError::AlreadyArmed` | `kayfabe-abi/src/eventnotify.rs:589` | |
| `SmcMode::Enabled` / `::EnablePending` / `::DisablePending` | `kayfabe-abi/src/smcmode.rs:92,96,98` | ABI-mirror completeness — **designed**, per `w258_dead_set_proposal.md` §3 |
| `GmmuVersion::Ver3` | `kayfabe-arch/src/lib.rs:232` | every `GmmuFmt::version()` in the tree returns `Ver2` (`kayfabe-chips/src/ga10x.rs:767,901`; `gh100.rs` implements no `version()`). Forward declaration, low cost |

⊘ **`Traffic::System` / `Traffic::Proc` (`kayfabe-core/src/lib.rs:235,237`) are deliberately
excluded** — they are never constructed either, but the type's own doc says so in terms
(*"No current signature consumes it … kept as the named rule the design ledger cites … deferred
to L1"*). **That is the pattern the rest of this section should be converted to**: an unconstructed
variant whose doc states it is unconstructed and why costs nothing, because it cannot mislead.

---

## 4. Checked and RULED OUT — do not re-file these

Recorded so the next auditor does not spend the same tokens. Each was a plausible candidate that
direct verification killed.

| candidate | why it is not rot |
|---|---|
| The 23 `kayfabe_shim_*` FFI exports | **All 23 are called** from `qemu/hw/misc/nvkvm/nvkvm.c`. A Rust-only reachability scan reports every one as dead; their callers are in another language. This is D4 of `scripts/orphan_gate.sh`'s own header, reproduced. |
| `birth_census::{GUEST_RING, DECLINED, NOT_ASKED}` | Written via a `match`-selected `&STATIC` at `kayfabe-isolate-host/src/rm.rs:1490-1494`. |
| ~18 `Audit` census fields in `kayfabe-vmm-qemu` / `-kvm` | Written via `Audit::bump(&field, &peak, delta)` (`lib.rs:481`, callers `:1700`, `:2273`); read via `g(&self.field)` (`:499-512`). |
| `DOORBELL_WITNESS_N` | The value is consumed from `fetch_add`'s own return (`rm.rs:1808`), not a later `load`. |
| `SetPageDirPolicy` | **Is** seated in production — `kayfabe-device/src/lib.rs:1227`. My first grep missed the `setpagedir::` qualifier. Only its *refusal enum* (R6) is dead. |
| `sweep_pt_tables_revoking`, `table_fingerprint`, `published_root`, `ceresolve`, `DoorbellInlineArm`, `DoorbellAsyncArm`, `selected_doorbell_{inline,async}` | All reached from `shim.rs`. |
| `#[cfg(not(feature = "host-isolates"))]` — 26 arms in `shim.rs` | **Not dead: it is the SHIPPING arm.** `scripts/build_qom_shim.sh:26-27` — *"the default is EMPTY, so a bench that does not set it gets byte-for-byte the archive master ships."* See S4 for the real question here. |
| `kayfabe-mocks`' unconstructed `RmVerb`/`VerbKind` variants | Test doubles; "no production caller" is their definition. |
| `slotnum.rs` vs `slots.rs` memslot allocators | Deliberate, documented, opposite-direction; already fixed at w641. |
| `ga10x.rs` in `kayfabe-device` vs in `kayfabe-chips` | Disjoint concerns, zero overlapping register constants. |
| GA106 `pci_device_id: 0x2504` in two places | Lookup key only; a mismatch fails loud (`ChipError::VbiosProfileMissing`), never guesses. |
| `SET_OBJECT = 0x0000_0000` repeated in three crates | Architecturally fixed at 0 for every NVIDIA pushbuffer class. |
| Doc comments spot-checked and found **true** | `plane.rs:2421` (`shadow_write` is the only port writer — 6 call sites, no other writer); `gpu.rs:406` (`take_guest_ram_pins`' single caller at `:3879`); `fbwin.rs:401` (`install_join`'s caller holds the plane lock — `plane.rs:2586-2591`); `kayfabe-mmu/src/lib.rs:326` (single reader at `:728`); `plane.rs:1-20` (`assert_disjoint` at `:5904`, runs at `:2248`); `reclaimtick.rs:43-53` (vCPU `try_claim_on_trap` at `shim.rs:16627` vs blocking `tick.spend()` at `:4322`). |

---

## 5. SUSPECTED — unverified, with the check that would settle each

| # | suspicion | settling check |
|---|---|---|
| **S1** | `kayfabe-mmu/src/gpga.rs:898` uses an **inclusive** end (`c >= region.base && c <= region.end()`) where `RegSpan::contains` / `RomWindow::contains` (`kayfabe-device/src/lib.rs:197,222`) are half-open. If `end()` is an exclusive sentinel this is an off-by-one on a containment test. | Read `gpga.rs:850-920` and the definition of `Region::end()`; confirm which convention it returns. |
| **S2** | `kayfabe-linux-raw/src/ioctl.rs`'s size helpers and `kayfabe-abi/src/capability.rs`'s `ControlEntry` table both encode "size for a command id" in different crates. Never cross-diffed. ⚠ CLAUDE.md records the twin of this: *"an ioctl number is not a length because its bits parse as one"* produced 2 672 phantom divergences. | Find an id present in both and compare the computed sizes. |
| **S3** | `gpga_is_one_reserved_object.md:4` (LIVE, 2026-09-10) says it *"supersedes the per-leaf join described in `fbwin.rs`'s module docs, which is scheduled for deletion by it."* `fbwin.rs` still implements and documents the per-leaf join with **no marker** (`grep -n 'superseded\|scheduled for deletion'` → 0 hits). Either the doc is stale or the deletion is queued — different classes. | `git log --since=2026-09-10 -- crates/kayfabe-device/src/fbwin.rs`, and check whether a newer RESUME doc tracks the deletion as pending. ⚠ `kayfabe-device` is under active edit; re-check before acting. |
| **S4** | **The shipping default arm may never be booted.** 30 of 159 `scripts/bench/*.sh` set `KAYFABE_SHIM_FEATURES=host-isolates`; `build_qom_shim.sh` says the empty default is *"byte-for-byte the archive master ships"*. If every boot that matters sets the feature, the 26 `cfg(not(host-isolates))` arms are shipped and **never measured** — the inverse of dead code, and worse. | For the seven scripts that call `build_qom_shim` without setting it (`scripts/run_full_suite.sh`, `scripts/bench/{provision_bench_tree,w326_all,w327_sweep,w328_all,w328_all2,w329_all}.sh`), check whether any actually **boots a guest** on the feature-less archive, or whether they only build. |
| **S5** | `KAYFABE_REV.txt` is a **third** revision source, last touched at `6c81a796` (2026-08-14), read by `scripts/bench/w326_arm.sh:30` and `w328_arm.sh:43`, with no regenerating script in this repo. | Find the provisioning step that writes it; if none writes it fresh, any bench quoting it is quoting 2026-08-14. |
| **S6** | `kayfabe-device`'s `census.rs`, `sweep.rs`, `faultbuffer.rs`, `mmuinval.rs` were **not swept** for store/recompute pairs. They are counter- and state-shaped, which is where class-2 and class-5 defects live. | A targeted read of those four files; 42k lines in that crate and they were the residue. |

---

## 6. Counts, and honest coverage

| class | verified entries |
|---|---|
| 1 — orphans that look live | 25 (R5, R11, R18–R21, plus the 20 in §2, minus overlap) |
| 2 — counters/fields/branches nothing writes or takes | 13 (R6, R7, R17, plus the 11 variants in §3, minus overlap) |
| 3 — docs that stopped being true | 8 (R2, R3, R9, R13, R14, R15 [25 files], R16 [4 docs]) |
| 4 — superseded env arms / knobs | 2 verified (R2, R12) + S4 |
| 5 — duplicated sources of truth | 4 (R1, R4, R8, R23) |

**STATUS census of `docs/design/` (173 files):** LIVE 88 · BUILT/MEASURED 20 · DECIDED/RULING 15 ·
AUDIT/GATE vocabulary (dated, but outside the house rule's four words) 9 · SUPERSEDED 6 ·
DESIGN/PLAN/QUEUED 5 · ANSWERED 4 · other terminal 1 · **NONE 25**.

### What was actually read, and what was not

**Read:** `scripts/orphan_gate.sh` (header + logic), `traces/gates/w258_dead_set_proposal.md`,
`traces/gates/orphan_sweep_w256/triage_all.tsv`, `qemu/hw/misc/nvkvm/nvkvm.c` (symbol
cross-reference only), `.github/workflows/ci.yml` step list, `scripts/build_qom_shim.sh`,
`scripts/ci_gates.sh`, and the specific regions cited in every row above. Design docs: opening
blocks of all 173, plus ~12 read substantially.

**Swept mechanically but not read:** all 2 491 `pub fn` declarations and all `pub enum` variants in
`crates/*/src`, via the reachability pass; every `static …: Atomic*` and every `Atomic*` struct
field.

**NOT covered — say so rather than imply otherwise:**
- `kayfabe-abi` (51 k lines, the largest crate) was treated as an **ABI mirror** throughout and was
  not audited for internal rot. `w258_dead_set_proposal.md` §3 already rules its unused surface
  designed; nothing here re-opens that.
- `kayfabe-device`'s `census.rs`, `sweep.rs`, `faultbuffer.rs`, `mmuinval.rs` — see S6.
- `kayfabe-gsp`, `kayfabe-crec`, `kayfabe-shell` — grepped only.
- `crates/kayfabe-isolate-host/src/bin/rmladder.rs` (15 k lines) was **excluded by design**: it is
  a probe binary, and ~60 `pub fn` on `HostRmBackend` exist solely for it. Those are a deliberate
  probe surface, not orphans, and listing them would be the "train readers to ignore the report"
  failure `w258_dead_set_proposal.md` §3 warns about.
- **No boot, no GPU, no `cargo` invocation.** Every claim here is from source and committed traces.

---

## 7. The rule this inventory argues for

★ **A gate that runs once is a snapshot, and a snapshot decays.** The w256 sweep was a good
instrument, its four self-defects were found and fixed at w258, a careful proposal said what to do
with its output — and then **the proposal's two structural recommendations were not implemented**,
so 13 new zero-caller items accrued unnoticed over ~400 commits. The expensive half was never the
164; it was that nothing was watching afterwards.

⇒ **The cheapest single fix in this document is R10**: commit today's `NO_CALLER_ANYWHERE` set as a
baseline, exempt the ABI mirror / OS seam / mocks by manifest with the reason recorded, and run
`scripts/orphan_gate.sh` in `ci_gates.sh` as a ratchet. Green on day one by construction, so it
never goes red on day two and gets disabled.
