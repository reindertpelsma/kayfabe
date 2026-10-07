# Compliance review: `claude/code43-gr-20261007` (runs 25–32) against the binding rules

**STATUS: LIVE, 2026-10-07.** This is a read-only review. It made no code edits and ran no build,
test or GPU. It compares `origin/claude/code43-gr-20261007` at `fe1e169a` with
`origin/codex/sdr-lut-20261006` at `faae81ab` (merge base `98275481`). The scope is every
non-trace change: kf-rm `vfguest`/`chanlink`/`display`/`inittables`; kf-abi
`eventnotify`/`capability`/`classes`/`versions`/gen; kf-disp `model` + layouts; kf-chan
`grtables`/`tmode`/`host`/`ring`/`translated`; kf-host `channel`; kf-qemu `chan`/`device`; and
kf-harness `kf-gr-tier`.

The rules checked are `AGENTS.md` *Rules*, `THE_ARCHITECTURE_v3.md` §0 and §0.3 (no literal in a
comparison, no capture-derived table) and §7, `V3_BUILD.md`, and `OWNER_RULINGS.md` §A, §F, §H
and §S.

Each finding is marked with a class and a confidence:
- **BLOCKER**: violates a binding rule.
- **SHOULD-FIX**: should be fixed, but breaks no binding rule.
- **NOTE**: for the record.
- **CONFIRMED**: shown in the code.
- **UNSURE**: the code alone does not settle it.

Paths are relative to the repository root. Line numbers are at `fe1e169a`.

**Counts:** BLOCKER 2 · SHOULD-FIX 7 · NOTE 11.

---

## BLOCKER

### B1. There is no native-oracle evidence for the GR tier, yet the code says it was measured. CONFIRMED (that it is missing); UNSURE (whether it was run)

- §S item 3 is binding. Each class/method set is validated with the native oracle "before any
  Windows run". §S item 7 requires the completion data and the interrupt relay to be verified
  and reported "in the native validation".
- The branch adds `crates/kf-harness/src/bin/kf-gr-tier.rs`, but no output from it is committed.
  `traces/windows_code43_walls_20261007/README.md:1191` says the results are "in the next
  subsection", but the file ends at :1209 with no such subsection, and there is no
  `*gr-tier*` log in that directory.
- Two code comments state the result as measured:
  - `crates/kf-qemu/src/chan.rs:1422`: "measured 2026-10-07 by `kf-gr-tier`, `completion_edges`";
  - `crates/kf-qemu/src/device.rs:3048`: "measured: GR0's notifier does not fire".
- `crates/kf-chan/src/grtables.rs:13-14` says the rows are what "the native oracle has validated
  on bare metal".
- This breaks AGENTS *Working rules*: "an inference is never written as evidence".

**Before run32:** run `kf-gr-tier` and commit its log, with its revision. It must show the USER
stamp, both objects, the engine-written semaphore and GP_GET, the completion edges, the named
refusals and a full release. If the oracle was not run, mark those comments as inferred.

### B2. The GR tier's class ids are hand-typed literals in comparisons; a generated source exists. Blackwell is refused. CONFIRMED

- The class ids are hand-typed: `crates/kf-chan/src/grtables.rs:28,30` (`0x902D`, `0xA140`).
- They are compared in `GrClass::of_class`/`id` (:47-62), used by
  `tmode.rs:546,614` and `host.rs:602`.
- Generated sources already carry them:
  - `kf_chip::classes::ClassSet::{twod, inline_to_memory}`, which `tools/derive_classes.sh`
    produces from `g_gpu_class_list.c` (`crates/kf-chip/src/classes.rs:188-308`);
  - `kf_abi::generated::matrix::CLASS_IDS_FERMI_TWOD_A` / `CLASS_IDS_KEPLER_INLINE_TO_MEMORY_B`
    (`matrix.rs:11382,11584`).
- `grtables::is_known_class` (:233) already uses the generated sets, so the pattern exists.

**Family effect.** The generated sets list:

| family | 2D class | I2M class |
|---|---|---|
| Turing, Ampere, Ada, Hopper | `0x902D` | `0xA140` |
| Blackwell | `0x902D` | `0xCD40` (`classes.rs:308`) |

- `HostRing::admit_gr_tier` requires every `GrClass::ALL` (`host.rs:602-618`).
  ⇒ On a Blackwell host with `KF3_KERNEL_GR_WORK=1`, the tier is refused by name. Every
  kernel-GR channel birth then fails with `NV_ERR_INSUFFICIENT_RESOURCES`
  (`kf-qemu/src/chan.rs:3829-3843`), even for 2D work.
- A Blackwell guest's `SET_OBJECT 0xCD40` hits `ForeignClass` (`tmode.rs:546`).
- The failure is a loud refusal, not silent misbehaviour.
- Turing, Ampere, Ada and Hopper would admit the tier: 2D and I2M both appear in their generated
  sets. Whether the host RM lists them is checked at run time (`supported_class_ids`).

This breaks AGENTS "Derive, never capture … all families are first-class" and §0.3.1 ("nothing
may test a raw … class id against a hardcoded constant").

**Fix:** key `GrClass` on `kf_chip::classes::Kind::{TwoD, InlineToMemory}` and resolve the id from
the host family's set. Admit each class independently, so that 2D does not depend on I2M. §S.2
("one class/method set at a time") permits a narrow method set, but it does not permit a literal
class id.

---

## SHOULD-FIX

### S1. The FERMI_TWOD_A method table is hand-typed, though its header is complete. CONFIRMED

- What is hand-typed:
  - `grtables.rs:97-134`: 17 method offsets, plus the `OPERATIONS` and `CPU_COLOR_FORMATS`
    enumerations;
  - `grtables.rs:147-176`: the 16 refused ranges.
- I checked every admitted offset and enumeration value against
  `ogkm-580.65.06 cl902d.h:537-981`. All match.
- The precedent for hand rows, `ttables.rs:5-11`, rests on *trimmed* CE headers. `cl902d.h` is
  not trimmed: it defines every method.
- No generated method source exists today. The compile-and-print pattern of
  `tools/derive_display_layouts.sh` could emit the offsets and `_V_*` values per version.
- The **choice** of methods is policy (§S.2) and stays hand-maintained. Only the numbers should
  be generated.

### S2. The `RULED_NOTIFIERS` indices are bare literals, not keyed by version; generated runs show drift. CONFIRMED

- The rows are `crates/kf-abi/src/eventnotify.rs:507-684` (e.g. `index: 182` at :659,
  `index: 197` at :665). They are matched without a version (`is_ruled_notifier`, :686;
  `kf-rm/src/inittables.rs:2374`).
- `kf_abi::generated::matrix::NV2080_NOTIFIERS_*` shows these values vary by version:
  - `AUX_POWER_STATE_CHANGE` is `0xb4` at 535.309.01 and `0xb6` at 545+.
  - `GPU_RC_RESET` is absent before 575.51.02.
  - `MAXCOUNT` is `0xb5` at 535.
- Today the drift is safe: on a 535 guest, AUX_POWER (180) is refused loudly. But the table
  cannot follow the Dg axis.
- The older lists in the same file use named hand constants (`NV2080_NOTIFIERS_POWER_RESUME = 194`,
  :311). This table does not even name its indices.
- **Fix:** resolve each row through the generated `ValueRuns` for the guest's version.

### S3. The "never posted by the real GSP" rows rest on a capture; the source argument is different. CONFIRMED (the basis); UNSURE (how §S.3 applies)

- Rows 12 GRAPHICS, 23/24/26 CE0/1/3, 120 and 122 are admitted because the real GSP posted none
  of them in vfio-8/9/10. That census was one RTX 4070 on Windows 580.88 (`eventnotify.rs:485-489`,
  540-580). §0.3.3 forbids treating a capture-derived table as truth.
- The source says who raises 12/23/24/26: the guest's **own** CPU-RM, from its non-stall ISR
  (`ogkm-580.65.06: kernel_ce.c:702`, `kernel_graphics.c:2631` → `engineNonStallIntrNotify`).
  The guest therefore sees them only if the engine's interrupt vector is relayed.
- The README (:1180-1184) records that a Translated ring's completion never raised a guest
  interrupt before this branch, and the new relay covers GR-tier rings only
  (`kf-qemu/src/chan.rs:4397-4400`).
  ⇒ For kernel CE rings, the guest arms CE notifiers that kayfabe never delivers.
- §S.3 says a completion event "is accepted only if it is derived from real host events".
- **Fix:** replace the census argument with the source argument, and state per row which relay
  delivers it. Ask the owner whether an arming that cannot yet be delivered on Translated CE
  rings satisfies §S.3.

### S4. `vfguest` hard-codes the status offset; the version table has it. CONFIRMED (hard-coded); UNSURE (whether any version differs)

- The hard-coded value: `crates/kf-rm/src/vfguest.rs:65` (`CONTROL_STATUS_OFF = 12`), written at
  :214.
- The version-aware value: `DriverAbiTable::rm_control_wire().status_off`, generated
  (`kf-abi/src/versions.rs:383,762`).
- The same file already takes `params_off` from that table.
- `display.rs:55`, `zbc.rs:36` and `inittables.rs:109` repeat the hard-coded value; that
  pattern is older than this branch.

### S5. Control ids, parameter sizes and values are hand-typed in kf-rm. CONFIRMED

- `vfguest.rs:68-81`: `0x2080220d/e`, `0x2080205a`, `RC_RECOVERY_*`, `POWER_SOURCE_AC`,
  `PARAMS_SIZE = 4`.
- `chanlink.rs:68`: `SET_DEFAULT_VASPACE = 0x00801812`, and the params-size literal 4 (:1434).
- Each is cited and correct for 580, but none is generated.
- On the same branch, kf-disp's new controls go through the generated TSV
  (`tools/derive_display_layouts.sh`; `crates/kf-disp/data/layouts-580.{65.06,159.04}.tsv`).
  kf-abi's gen `ConstReq` (as used for `NV01_EVENT_KERNEL_CALLBACK`) could do the same for kf-rm.
- Precedent: `kf_abi::submit` hand control ids.

### S6. `PERF_GET_POWERSTATE` returns a value in a power area that §S keeps host-owned. UNSURE (needs an owner ruling)

- `vfguest.rs:174` answers `AC`.
- The value is NVIDIA's vGPU-guest body (`kern_perf_ctrl.c:293-308`), so it comes from source.
- §S.1, however, says queries in a stubbed area are "refused or reported absent, never filled with
  invented values". §S.6 says power stays a host-owned stub.
- No ruling names this control. Cite one or refuse it.

### S7. A security-policy (capability) change has had no owner review. CONFIRMED

- `kf-abi/src/capability.rs:1531-1543` admits `NV01_EVENT_KERNEL_CALLBACK` for **every** guest
  at 580.65.06 or later, Linux included. It was measured on Windows 580.65.06 only.
- §F: "Security-policy changes need explicit owner review before merge."
- On the safety side:
  - the params are never decoded (`versions.rs:1511-1513`), so the guest pointer is never read;
  - the change is an object-graph edge only.

This must go to the owner before any merge. It does not block experiments on the branch.

---

## NOTE

- **N1. The privilege boundary holds. CONFIRMED.**
  - Only the birth path sets `born_user` (`kf-host/src/channel.rs:886-890`).
  - `assert_user` refuses a missing or privileged stamp (:177-183; tested at :1756-1773).
  - The tier calls `assert_user` first (`kf-chan/src/host.rs:589-594`).
  - Host objects come from the fixed list, filtered by the host family's generated set and the
    host's `supported_class_ids`. The only verb is `alloc_engine_object` on kayfabe's own USER
    channel. No privileged verb is used, and no host action is authored from guest bytes.
- **N2. Completions are not forged, and no CPU executor exists. CONFIRMED.**
  - GP_GET is a GPU `SEM_EXECUTE RELEASE_WFI` through the perimeter (`host.rs:239-255`).
  - `author()` runs only once the host fence has been `reached` (`host.rs:1289-1293`).
  - The guest GR0 vector is raised on the worker after that point (`chan.rs:4397-4400`,
    `device.rs:3051`), never on a vCPU.
  - The relay is pump-derived: the pump is woken by `FIFO_EVENT_MTHD`. It is not a direct
    NSI→vector map; this matches §S.7's wording, if loosely.
- **N3. Every admitted 2D row is address-free, and every trigger, inline data method and address
  register is refused. CONFIRMED.**
  - Nothing can reach memory through the tier (`grtables.rs:147-176`; test :272-289).
  - `Field::Word` re-authoring is the identity (:226), which is equivalent to copying a 31:0
    field.
  - These coordinates and scales will set the footprint once a trigger is admitted, and must be
    bounded then.
- **N4. Software-subchannel inert binds** (`tmode.rs:629-640`) use the generated per-family sets.
  They are off by default (`KF3_SW_SUBCH_INERT`), as ruling 4 requires. The hardware behaviour is
  inferred and untested, as the ruling itself says.
- **N5. The refused-segment log is bounded.** It holds at most 128 guest words, once per ring,
  because a refusal is terminal (`ring.rs:125-143`). Total volume grows with the guest's channel
  churn.
- **N6. The display additions are derived per version.**
  - Layouts and constants come from ogkm for 580.65.06 and 580.159.04.
  - Any other version gets `model_for → None`, never a guessed layout (`kf-rm/src/display.rs:140`).
  - Event bindings are bounded at 64 objects × 32 events (`kf-disp/src/model.rs`).
  - Hotplug state comes from kayfabe's own monitor, which is §S.2 implemented for real.
  - `PRE/POST_MODESET` and `ACPI_SUBSYSTEM_ACTIVATED` are accepted with no effect, citing §S.1.
  - `IMP_ENABLE` GET returns FALSE, which is coherent with `mode_possible`.
- **N7. Small literals are left in display.rs and the harness.**
  - In `display.rs`: `0x0000_0005` (NV01_EVENT, :187) and `NOTIFY_INDEX_MASK = 0x03ff_ffff`
    (:179). The mask is correct against `nvos.h:418-436`; generated `nvos.rs` lacks both.
  - In the harness, `kf-gr-tier.rs:51` hard-codes `GR0_NOTIFIER = 12`, although the generated
    `NV2080_NOTIFIERS_GR0` exists.
  - The harness also asserts `classes == [0x902d, 0xa140]` (:350), so it fails on Blackwell
    (see B2).
- **N8. The RC-recovery stub follows §S.1, which names `SET/GET_RC_RECOVERY`.**
  - The initial `ENABLED` (`vfguest.rs:120`) is the VFIO passthrough answer, i.e. a capture.
    Treat it as a documented default.
  - Stale text: the module table says "always `DISABLED`" (:27), and `RcRecovery` says "Only one
    value exists today" (:82). The ⊘ marker above them covers the module table only.
- **N9. `chanlink.vas_parents` outlives a Device free** (`chanlink.rs:1565-1566`).
  - Its entries are removed only when the VA space itself or its client is freed.
  - A Device handle that is freed and then reused could accept `SET_DEFAULT_VASPACE` naming a
    stale VA space.
  - The effect stays inside the guest, and growth is bounded by the client's lifetime.
  - UNSURE: whether the guest RM sends a FREE for each child.
- **N10. The internalFlags privilege check is hard-coded to the 580 layout and is skipped when the
  decode returns None** (`channel.rs:323`; older than this branch). `raw_alloc` converts the reply
  to the bench layout (`kf-host/src/lib.rs:888-889`), so `V580` is the right wire. The new tier's
  doc relies on that check. UNSURE: whether `internalFlags` is among the converted fields on
  595.91.07.
- **N11. No new `unsafe`, and no new raw addresses outside `*_unsafe.rs`.** The tier's addresses
  go through `tspace_unsafe` (`WindowAddr`, `put_host_sem_execute`). Both flags are off by
  default: `KF3_KERNEL_GR_WORK` needs `KF3_KERNEL_GR_CE` (`chan.rs:664`, :3817).

## Answers to the five checks, in short

1. **Derive, never capture.** The 2D/I2M class ids (B2) are hand-typed, though a generated source
   exists. Hand-typed with no generated source yet: the 2D method table (S1), kf-rm control
   ids/sizes/values (S4, S5) and the `NV01_EVENT`/mask literals (N7). The notifier indices (S2)
   have generated runs that drift. kf-abi `NV01_EVENT_KERNEL_CALLBACK` (0x78) and the display
   layouts are generated.
2. **Families and dies.** Nothing is wired to AD104, 595.91.07 or one guest version, except the
   captured notifier census (S3) and the initial RC value (N8). Blackwell is refused by the GR
   tier (B2). Turing, Ampere, Ada and Hopper pass on the generated sets.
3. **Per-driver-version layouts.** Both audited guest contracts (580.65.06 and 580.159.04) are
   covered for display. Other versions are refused (N6). The exceptions are the status offset
   (S4) and the notifier indices (S2).
4. **Hostile guest.** No unbounded read or emit, no host action authored from guest bytes, the
   channel is USER-asserted, and nothing is forged (N1–N3, N5).
5. **Stubs under §S.**
   - RC recovery and notifiers 2/4/33/34/43/44/45/139/157/158/182/197 are cited.
   - The HDCP absence is kept coherent (`eventnotify.rs` row 34).
   - `PERF_GET_POWERSTATE` needs a ruling (S6).
   - The CE/GR armings rest on a capture rather than a host-event derivation (S3).

## Should the GR branch change before more runs?

Yes, before any run with `KF3_KERNEL_GR_WORK=1`:
- commit the `kf-gr-tier` native-oracle log, and correct the "measured" comments (B1);
- derive the GR class ids from `kf_chip` and admit each class independently (B2).

S2 and S3 should be fixed before the notifier table grows further. S7 needs owner review before
any merge. The other findings do not block run32.
