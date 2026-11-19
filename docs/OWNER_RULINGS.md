# Owner rulings — the decisions that govern kayfabe v3 work

**STATUS: LIVE, 2026-11-18 (§AX (2026-11-17): standalone trust is git only by default, the lift is explicit, what the user passes is trusted, and a user-passed mirror refines §AW.2; §BA: propimages are configuration, standalone needs only git and official URLs, one stored image per propimage, no licence time checker, one-way clock rule, token revocation; §AZ: proprietary images and SPEC leave the required release gate and run as an app lane; 2026-11-17: §AW: pins are deployment data, identity pins strict-only; 2026-11-16: §AU: the uniform clock rule; 2026-11-15: §AT: the measured Windows profile is the default, reversing §W's software-runlist "undecided"; §AP records the whitepaper answers; §AM added 2026-10-18: the vidmem scrub policy; §S added, then the BAR0-trace exception and the `PERF_GET_POWERSTATE` confirmation; earlier rulings dated in place); §T added 2026-10-08 (filtered recovered directives); §X added 2026-10-08 (non-stall interrupts); §Y added 2026-10-09 (diagnostic BAR0 trace mode, numbered §X on its branch, the VFIO tracer); §AB added 2026-10-10 (T-space hardwired, privilege rules for mappings).** Every ruling the owner made in the 2026-09-25 … 09-30 working sessions,
with its date, so work can resume from the repository alone. The architecture itself is in
`docs/design/THE_V3_PLAN.md` and `THE_CONSTRAINTS.md`; this file records *decisions* on top of it.
Where a ruling was later refined, the refinement is listed under it. A ruling's date is part of its
citation: ask whether its reason still holds before relying on it.

## §BA. Propimages are configuration, standalone needs only git and official URLs, one stored image per propimage, no licence time checker, and other 2026-11-18 rulings

**STATUS: LIVE as rulings, 2026-11-18; owner decisions (stated in conversation, recorded by the coordinator from its notes). Each item says what is a rule and what is only a plan. DESIGN-ONLY for the config schema and the generic stage: nothing in items 1 and 2 is implemented yet; the refactor is queued.** Where an item restates a rule already in `AGENTS.md`, it is repeated here only to give it a date and a reason.

<!-- src: owner, 2026-11-18, conversation with the coordinator (propimage config, from-scratch standalone, one image per propimage, no licence time checker, clock rule, gap matrix, test-first, pc recovery, Vast offers, public screenshots) -->

1. **Propimages are configuration, not names in code.** The images called A, B, C and D are configuration material. The code has no
   concept of "image A": a propimage is a configuration entry (id, kind Windows or Linux, image source and trust anchor, identity or
   seed rule, launch arguments, its apps with their rows, its gate rows) and **one generic stage** iterates the entries. One image
   holding all the apps, or a subset spread over several images, must both be expressible. No new per-image stage name, environment
   variable, script or test that names A, B, C or D: add a configuration entry instead. *Reason:* MEASURED on the 2026-11-18
   acceptance tree: per-image stage names and environment names, and more than 20 files naming one image; third parties must be able
   to bring their own images. The repository holds the schema, the validator and the generic stage. The concrete entries of the
   project's own release gate (with their object-store keys) are operator deployment configuration, not repository code, because they depend on the
   storage layout. *Plan, not done:* the refactor, with an equivalence test that the generated stage plan is identical before and
   after.
2. **Standalone is reproducible from git and official URLs only.** A third party must be able to run the whole standalone run from
   the repository plus official URLs (each with an official anchor: upstream checksum, signature, attestation or pin), their own
   inputs given by configuration, or software built from source by the repository's own recipe. No input may have our object store as
   its only source, and the standalone plan may name no host of ours. Everything open is built from scratch. Proprietary software is
   obtained by the user under their own licences and accepted agreements, and each proprietary app must be addable, testable and
   includable by pointing the configuration at an installer (with the repository's silent-install recipe) or at a pre-installed image;
   the licence input is a user-supplied file or environment value that the tooling never records or prints. An app that is configured must pass
   under the strict zero-skip rule; an app that is not configured is listed as "not in this config", never skipped silently and never
   counted as a pass. The project's own release gate is the project's own configuration with all of them. *Reason:* the from-scratch
   requirement of the standalone run (see `AGENTS.md`, *Read first*, and the pins ruling §AW). *Known gap, measured in the 2026-11-18
   audit of the run manifest:* a few inputs (the public Windows apps bundle, the SPECviewperf repack, the proprietary images, the signed
   display firmware and the MATLAB and Unreal artifacts) have only our store as a source; for a third party these are their own inputs
   by configuration, and a lint that the generated plan names no host of ours is queued.
3. **One stored image per propimage.** In long-term storage there is exactly one image for each propimage, containing all its apps.
   Layer checkpoints (a base plus one app at a time) stay on the maintainers' PC only. For safety the PC is never the only copy
   (an earlier owner rule: the PC is unreliable), so one rolling copy per propimage may be kept at a fixed path, replaced and
   read back before the old one is deleted; at the end that one object is the final image. No per-layer copies side by side. The blank
   Windows base is a separate image. *Reason:* storage and clarity; the earlier images are single images too.
4. **`state.json` has no licence time checker; the clock policy is one-way.** No `valid_until`, no licence-expiry check and no
   clock-versus-licence validation in `state.json`, the propimage tooling, the Worker or any gate; a product's end date is never
   written into code, a state file or a public document. The recorded-clock mechanism stays as it is. *Reason:* licence handling
   does not belong in public code, and the pinned clock was already an accepted, private decision. The clock rule is **one-way:
   internet on means the real clock, and that is enforced in every launch path. Internet off does not force a fake clock.** Whether
   an offline guest runs a recorded clock or the real one is the user's or the Worker's policy, and a third party's policy may differ.
   *Measured:* `clocknet_assert.py` already enforces the one-way direction and states that a real clock with the network off is
   fine. *Design intent, not built:* an optional policy block in `state.json` that keeps clock and network independent (defaults equal
   to today's behaviour; the gate pins them), documented in the howto with a schema reference. This refines §AU items 1 and 2.
5. **Gap matrix: BROKEN target is 0; the managed-memory patch tier is optional and later.** Every BROKEN row of the GPU-gap matrix
   needs a closure path: an in-tree fix, an optional patch tier (host and guest kernel or driver patch), an owner decision, or
   not-applicable-by-design. The target count of BROKEN rows is 0. UVM managed memory (demand paging) is an *optional later patch
   tier*, not a core requirement. *Reason:* adding passing apps for weeks without fixing kayfabe is inefficient; the metric from now is
   confirmed kayfabe bugs found and kayfabe-code fixes merged, not rows or passes.
6. **Reproduce a suspected bug with a cheap GPU-free Rust unit test first.** Before a hardware run or a theory, write the failing
   unit test (a replay or a model test, with a mutant that proves it can fail) and only then fix or measure on a GPU. *Reason:* a
   wrong hypothesis costs far more than a test; it keeps hardware time for what only hardware can answer.
7. **A bare-metal or VFIO control decides whether a failure is kayfabe's.** Run the same case on the host or through VFIO first. If
   it passes there and fails in the guest, the defect is kayfabe's; if it fails there too, it is not (`AGENTS.md`, *Verification*).
   A harness defect caught by a host control is recorded as a harness defect.
8. **The maintainers' PC is the VFIO and performance reference machine, and a wedged PC is never normally rebooted.** When the GPU
   or driver is wedged (a driver unload or reset hangs, processes stuck in uninterruptible sleep), do not pile on reload attempts and
   do not use `reboot`, `shutdown -r` or `reboot -f`: use the SysRq sequence `s` (sync), `u` (remount read-only), `b` (restart), one
   step at a time. *Reason:* MEASURED 2026-11-16 and 2026-11-18: a normal reboot hung in the kernel's device shutdown waiting on the
   stuck GPU device and needed a power cycle; SysRq `b` skips that path. Reload or reset is only for a GPU that is in error but still
   responds. Never on a healthy PC. Sealed images are read-only, so only resumable work is at risk.
9. **Rented offers are chosen by the final total price, with no regional preference.** The offer ranking prices rental, bandwidth
   and storage into one total and ranks by what the run costs; the split by component does not matter. Data comes from the object
   store behind a global edge network, so downloads are not region-bound, and a region preference only shrinks the pool. The one
   region rule is the Worker's denied-countries list (trust policy), which stays. *Checked 2026-11-18:* no region preference exists in the offer
   picker or the Worker's offer code.
10. **Screenshots of proprietary apps may appear in public posts, never in the repository.** A screenshot is acceptable for a public
    post if the account or user name, e-mail address, licence or serial strings and machine names are blurred and the owner reviews
    each one before use. Screenshots stay out of the repository and out of `traces/`. *Reason:* the earlier "no screenshot leaves the
    box" was a privacy rule, not a legal one; it is relaxed to "scrubbed, owner-reviewed, post only".
11. **Cached logins stay in a sealed propimage; safety is vendor-side revocation, verified online.** Offline operation needs the
    cached login, so removing it locally defeats the image. Protection against a leaked image is to make the login dead on the
    vendor's server after the seal, and to verify that with an online check on a throwaway overlay. *Reason and measurement:*
    `docs/howto/recreate-the-proprietary-image.md`, *Token revocation and the online token check* (MEASURED 2026-11-18). An image is
    not used on a rented box or uploaded until its check has passed.

## §AZ. Proprietary images and SPEC leave the required release gate; they run as an invoked app lane, required only when a release touches core paths, 2026-11-18

**STATUS: LIVE as a ruling, 2026-11-18; owner decision (stated in conversation: "maybe the plan is to remove images and SPEC from release requirement, so we can launch them manually, or invoked through worker"; recorded by the coordinator). DESIGN-ONLY for the Worker: the Start kind `app` is not built and nothing is deployed. 2026-11-18 (implemented GPU-free, `docs/design/V3_APP_LANE_PLANNER.md`): the planner's `core_paths` data, `appLaneRequired`, the app leg, the first-release and rotation modes and the daily-cap refusal exist as offline code; the release criteria do not read them yet.** The cost model and the release plan behind it are in the coordinator's notes and are not yet a repository document (a design note follows this ruling); the discovery-versus-gate split below follows the owner's remark that apps find bugs in the unknown while conformance suites prove that a bug is solved.

1. **The required release gate is the reproducible set.** It consists of the GPU-free layer (unit, replay, model and fuzz tests, the page-table oracle fuzz, the coverage meter), the per-family smoke tier, and the full-suite differential (Vulkan CTS and the other open suites run host against guest, only differences reported, a known-limits file for documented limitations). Every input of the gate is open or fetched from an official URL, so a third party can run it (the owner's standalone requirement of 2026-11-18, recorded by the coordinator).
2. **The proprietary images and SPEC are not part of the required gate.** That covers every propimage (today A, B, C and D; the owner's rule of 2026-11-18, recorded by the coordinator and not yet folded into this file, is that they are configuration and not names in code) and the SPECviewperf stage. Reasons (measured or stated): their verdicts are often inconclusive or harness-level, a flaky host can fail a leg, they cost about 30 percent of the release, and a third party cannot reproduce them without the owner's proprietary inputs. `scripts/ci/vast_payload.py` already lists the image stages (`MANUAL_ONLY_STAGES`) as manual-only.
3. **The app lane.** They run as an invoked lane: from the Worker admin page (a start kind `app`, to be built) or by hand, on boxes chosen by a profile that prices bandwidth by the real gigabytes of each role and is free to pick any die with enough VRAM (not pinned to a family). It may also run on a schedule between releases, and it runs on the owner's pc when that is free (the pc is the discovery machine: VFIO controls and perf ratios). A run records the commit, the die, the host driver, the rows and the verdicts.
4. **The release record shows the app lane.** Each release states, per image, the last app-lane run: commit, date, die, family, result. It is information, not a criterion, except under item 5.
5. **Conditional requirement.** If the change since the previous release (the batch PR's diff against master) touches any CORE PATH below, the app lane must have a green run, or a run whose only reds are listed known breakages, on a tree that contains the change, covering each image on at least one family, before the batch PR merges. If no core path is touched, the app lane is not required.
   **Core paths (exact list; globs relative to the repository root):**
   - `crates/kf-*/src/**` except `crates/kf-harness/**`, `crates/kf-rawclient/**` and `crates/kf-trace/**` (the kayfabe product crates, including their generated tables under `src/generated/`);
   - `crates/kf-chip/**` (family and die tables and the per-family descriptors);
   - `cuda/walk/**` (the walk kernel, its header and the committed PTX);
   - `qemu/hw/misc/kf3/**` (the device, its properties and their defaults);
   - `broker/**` (the display broker) and `firmware/**` (the boot-display driver);
   - `scripts/bench/windows/propimage.py` and `scripts/bench/windows/clocknet_assert.py` (the launch arguments, the clock and network rule of a Windows guest).
   Not core paths: `docs/**`, `site/**`, `ci/**`, `scripts/ci/**`, other `scripts/**`, `traces/**`, tests only (`crates/*/tests/**` alone), and CI-only changes. The list is data (a `core_paths` array in the release configuration read by the planner), so it is edited in one place; the planner never names an image.
   **First release (owner, 2026-11-18: "my plan is still first release to run all of A to D").** The first release runs the app lane in full: every image (A, B, C and D as configured) on every architecture family of the release matrix, not only the rotation of item 6 and not only the minimum of this item. It is the baseline against which later rotations are read. Cost and die choice come from the planner (any die with enough VRAM per image, offers by final total price); the cost of this pass is estimated before it is started and is a decision for the owner if it exceeds the daily cap. DESIGN-ONLY until the app lane exists; until then it is run by hand with the same coverage.
6. **Rotation.** Each release runs the images on rotating families so that every image meets every family within four releases; a rotation table records the last run per image and family. A change to an architecture-specific path (a per-family descriptor, a channel route, the walker) escalates to every image on the affected family.
7. **The discovery lane is separate and non-gating.** Real applications and production workloads run between releases, chosen by the coverage gap (new RM control ids, new refusals, new Xid lines: a novelty signal), with automatic capture of the kf3 census window, the host log and the guest events; every novelty is triaged with a control (VFIO, native Windows or the host) and becomes a GPU-free replay test and a gate row.
8. **What this does not change.** The standalone run still lets a third party add their own proprietary apps and images by configuration (the owner's rule of 2026-11-18). Nothing in this ruling edits how an image is built, and a PASS of the required gate is still a statement only about the gate.
## §AX. Standalone trust is git only by default; the lift is explicit; what the user passes is trusted, 2026-11-17

**STATUS: LIVE, 2026-11-17; owner rules (stated in conversation, recorded by the coordinator); step 1 of the plan is built GPU-free, no box ran it.**
Design, the list of what was changed and what was not: `docs/design/V3_STANDALONE_TRUST.md`. Code: `scripts/ci/kf_trust.py`,
`scripts/ci/trust_manifest.json`, `scripts/ci/trust_lint.py`. Refines §AW.2 (the supersession note stands in that sentence).

1. **Two actors.** The OPERATOR runs their own Worker, boxes and object store: implicit trust in their own storage and pipeline, and a pin is
   deployment data that guards against staleness, corruption and accidents (§AW), not a security anchor. The THIRD PARTY dislikes opaque
   blobs: a standalone run trusts git (the reviewed repository) and nothing we build or host.
2. **The default of a standalone run is "trust git only".** What the repository downloads by itself must be verified by an anchor that does
   not depend on us. The anchors allowed for a repo-defined default: (1) the official upstream checksum or signature, fetched from the
   upstream origin; (2) a GitHub attestation, verified (NOT BUILT: the helper refuses it by name, `TRUST_ATTEST_NOT_IMPLEMENTED`); (3) a pin in
   git of an upstream-authored identifier that anyone can re-check against upstream: a git commit or tag id, a vendor-published checksum, a
   registry digest, or the sha256 of a SOURCE input. **Not allowed:** a sha256 in git of an artifact WE build or host (class
   `git-sha256-of-our-own-artifact`; the lint fails on it and the helper refuses it).
3. **Lifting is explicit and recorded.** Only a named flag (`--trust-lift REASON`, `KF_TRUST_LIFT=REASON`) lifts the default; a failed
   check then becomes anchor `unverified-lifted` with the computed sha256, marked non-gating. Nothing implies the lift: not reachability,
   not a URL scheme, not the absence of a Worker.
4. **What the user passes is ALWAYS trusted.** A path or URL the user passes explicitly (an image, a `.run`, a mirror, a URL file) is a user
   input: its sha256 and size are recorded, `--expect-sha256` is the user's own opt-in and is enforced in every mode, no verification is
   needed. The newest statement decides the conflict with §AW.2: **a user-passed mirror, URL or path for a repo-defined artifact is a user
   input, trusted and recorded.** Strict mode (an operator who sets `KF_TRUST_MODE=strict`; having the Worker's instruction set it is DESIGN INTENT, not wired in this
   change) additionally holds a user object to the pin the operator keeps (for example the SPECviewperf archive's pin). The Worker's
   `/bench-data` route, which has no URL file, already keeps that pin.
5. **PENDING OWNER CONFIRMATION.** The owner agreed to anchor (3) for SOURCE inputs. This change also accepts, and implements, the sha256 of
   an upstream BINARY that upstream still serves byte-identically (measured from the upstream host, no vendor checksum behind it). Each such
   entry is marked `pending-owner-confirmation` by the helper and listed by `python3 scripts/ci/trust_lint.py --pending` (11 names on
   2026-11-17). If the owner rules against it, those entries lose their anchor and their runs need the lift or a user-supplied file.
6. **Not decided here:** whether `qemu_source.py --from-file` (today still held to the pin, as §AR said) should become a plain user input;
   the attestation route (step 2) and the CI workflow that would build and attest what only we can build (steps 5 to 7 of the plan).

## §AW. Pins are deployment data: identity pins are strict-only; a standalone run records and never refuses on them, 2026-11-17

**STATUS: LIVE, 2026-11-17; owner rule (stated in conversation, recorded by the coordinator).** Design, audit and the run-your-own-deployment
recipe: `docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md`. Built GPU-free; no box ran it.

1. sha256 and size **identity pins** (a fixed sha256 of "the" image, fixed object names) are deployment data for our Worker and for strict
   or CI runs only. A **standalone** run (a user running `vast_payload.py`, `propimage_apps.py` or the generic runner themselves, permissive)
   must not require pins for images, artifacts or apps: someone may have built their own Windows images, compiled their own software
   (for example Unreal), or spread applications over different images. The proprietary images are not strict for standalone.
2. **Integrity checks stay** (a pack manifest's member hashes, the seed binding, cache-key consistency). Pins on what the repository
   itself downloads (swtpm, libtpms, noVNC, wheels, install media) are supply-chain integrity and hold in every mode. **[Refined by §AX,
   2026-11-17: they hold as integrity checks on what the repository downloads BY ITSELF; a mirror, URL or path the user passes for such an
   artifact is a user input and is trusted and recorded, not refused, and the pin that decides what the repository's own download is must
   be an anchor that does not depend on us (see §AX.2). The coordinator's audit cites this sentence as "AW.4": in the tree it is §AW.2,
   and rule 4 of `docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md`.]**
3. A standalone run **records** the computed sha256 and size of each user-supplied object in its evidence instead of refusing on a mismatch;
   `--expect-sha256 NAME=HASH` lets a user opt in. One mechanism: `propimage_runspec.pins_enforced(spec)`, true only for a strict spec.
4. Anyone can **run their own deployment**: build artifacts with the repository's scripts, publish to their own object store, write their own
   config and pins. Storage is plain S3 semantics (R2 is S3-compatible; no R2-only features; presign with any S3-compatible API).
5. The Worker's own behaviour (`ci/vast-broker/propimages.json`, strict always) does not change.

## §AU. The guest clock depends only on the network; a recorded clock is never earlier than the software loaded into the image, 2026-11-16

**Refined 2026-11-18 by §BA item 4 (read this first):** the clock rule is one-way. Network on means the real clock. Network off does not force a recorded clock; that is the user's or the Worker's policy. Items 1 and 2 below say "network off runs a recorded clock" and "WARN" for an offline real clock; read them as the project's default policy, not as a rule a third party must follow.

**STATUS: LIVE, 2026-11-16; owner rulings (stated in conversation, recorded by the coordinator).** Design and measured/code-read
split: `docs/design/V3_CLOCK_RULE.md`. Implemented GPU-free in `scripts/bench/windows/clockrule.py` and `win_vm.sh`; nothing here has
run on hardware yet.

1. **Network off** (slirp `restrict=on`, only the ssh forward) runs a **recorded** clock; **network on** (`restrict=off`) runs the
   **real** clock. Whether an image is open or proprietary does not matter. An offline image needs no current time; a moving clock
   only adds ways for it to break. A guest with network needs the real time.
2. A recorded clock with network is always refused. An offline guest on the real clock (no record yet) is a WARN, with one
   switch (`WINVM_CLOCK_STRICT=1`) that the owner can flip later to refuse it.
3. **A recorded clock is never earlier than the creation time of software loaded into the image** (SPECviewperf bundle on the open
   LTSC image, a guest driver installer): clock floor = max(recorded, newest created_utc + 1 h). Our scripts and test/input assets do
   not count. A proprietary image uses exactly the clock of its `state.json` and no floor.

## §AT. The measured Windows profile is the default; `windows-support=on|off` is the one opt-out, 2026-11-15

**STATUS: LIVE, 2026-11-15; owner rulings (stated in conversation, recorded by the coordinator).** Implements Stage 1 of
`docs/design/V3_FLAG_INVENTORY.md`'s flag cleanup (draft PR `claude/flags-stage1-20261115`). Nothing here has run on hardware yet:
the first proof is the Windows gate on a rented box with NO env file (`docs/design/V3_WINDOWS_SUPPORT_PROFILE.md`, *Status*).

<!-- src: owner, 2026-11-15, flag-cleanup conversation: "Can you set these flags as default. To start windows you dont need an env file. Then the defaults are wrong." and "by default kayfabe should run both linux & windows out of the box, unless it's explicitly told to be linux only in the command line" -->

1. **2026-11-15, owner: set the measured Windows profile as the default.** The 18 flags of the recorded Windows Code 0 holds
   (`KF3_WIN_USER_CHANNELS_PASSTHROUGH`, `_WIN_TWIN_DEFAPI_OBJECT`, `_KERNEL_GR_CE`, `_KERNEL_GR_WORK`, `_TRANSLATED_CE_RELAY`,
   `_DEFERRED_API`, `_PREEMPT_BIND_PROBE`, `_ZCULL_BIND_PROBE`, `_ASYNC_PREEMPT`, `_DISPLAY_CTRL_PROBE`, `_DISPLAY_IMP_ENABLE`,
   `_SW_RUNLIST_PROBE`, `_SW_RUNLIST_HOST_OWNED`, `_GFX_POOL_PROBE`, `_MEMORY_LIST_PROBE`, `_DISPLAY_SDR_COLOR`,
   `_DISPLAY_LUT_MIRROR`, `_DISPLAY_ILUT_OFFSET_256`) are hardwired **together**. What was measured: the Windows gates PASSed with all 18
   on (RTX 4090 `a76d1397`, RTX 3090 `006d845a`, RTX 5070 Ti `54410b7d`, and an Ada die-coverage run: three GPU families; `windows_apps` 10/10 on the 4090). Individually ablated
   (2026-10-10/11, idle desktop only): `TIMER_MAP` (not in this set) and a subset of the 18 (see `V3_FLAG_INVENTORY.md`); **not** ablated:
   `ZCULL_BIND_PROBE`; needed only INFERRED: `KERNEL_GR_WORK`, `SW_RUNLIST_PROBE`, `MEMORY_LIST_PROBE`; idle-only: `TRANSLATED_CE_RELAY`,
   `ASYNC_PREEMPT`; `DISPLAY_SDR_COLOR` was "not needed to boot" (colour fidelity). Those passes ran with the OLD, process-wide flags; the
   per-declaration form shipped here (identity-gated per alloc, per space and per page) has **not** run on hardware. "To start Windows you
   don't need an env file; then the defaults are wrong."
2. **Both OSes out of the box.** "By default kayfabe should run both Linux and Windows out of the box, unless it is explicitly told to be
   Linux-only on the command line." The compromise: each behaviour is gated on the guest DECLARING the Windows driver identity (owner
   rule D4: keyed on declared state, never a flag), so Linux is unchanged by construction; one coarse device property
   `windows-support=on|off`, **default on**, is the only opt-out (`docs/design/V3_WINDOWS_SUPPORT_PROFILE.md`). "Windows-only" is not worth
   a switch. The rest of the `KF3_*` flags stay, documented for power users; the env-read allowlist lint comes later (Stage 3).
   **The declaration is the guest's own bytes (rule D4): it selects a compatibility profile and is not a security boundary.** An honest Linux
   guest never reaches the profile; a guest that declares Windows 580.88 reaches about fifteen paths that used to need an operator env flag
   (user-work twins and the user ring, the 64 KiB-floor spaces, the host deferred-API object, the kernel-GR tier, the copy-engine relay,
   memory-list descriptors, async preempt with cross-client re-enable, host-owned scheduling, the colour decoder). No path into another VM or
   the host was found (read, not tested); a dedicated hostile-guest review is a follow-up. `windows-support=off` is the hardening for
   Linux-only or multi-tenant hosts.
3. **`KF3_SW_RUNLIST_HOST_OWNED` is turned on with the rest.** §W's "stays off and undecided" is reversed (see §W): "undecided" was never a
   decision to keep it off.
4. **Fabricated answers become product behaviour, named as such.** The profile includes answers no host query produced (class (c) in the
   flag inventory): the `GFX_POOL_QUERY_SIZE` sizes (`kf_abi::gfxpool::experimental_query`), `NV_OK` with nothing bound on the host for
   `GR_CTXSW_PREEMPTION_BIND` and the cross-client `GR_CTXSW_ZCULL_BIND`, the six echoed NV0073 controls, the software-runlist submit
   `NV_OK` (contents ignored) and `IMP_ENABLE` TRUE. They are served only to a declared Windows guest; code and docs say "FABRICATED". They
   claim no GPU work (§S: no forged completion).
5. **Not decided here:** the colour path (`DISPLAY_SDR_COLOR` with its two decoder switches) is keyed on the declaration and shown to a Windows
   guest only; whether a Linux guest should also get it is **not decided** (the Linux colour audit keeps `KF3_DISPLAY_SDR_COLOR=1` as an explicit
   lab override). Stage 2 (delete dead and old-experiment branches), the lint and display-mode properties are later stages.

## §AS. User flow: compile QEMU, use your own tools; no new CLI; module and hypervisor independence are priorities, 2026-11-15

**STATUS: LIVE, 2026-11-15; owner rulings (stated in conversation, recorded by the coordinator). Feasibility of the QEMU module is NOT yet studied.**

<!-- src: owner, 2026-11-15, guesthouse comparison conversation -->

1. **Integrated, not another manager.** Users do not want another CLI to manage VMs. kayfabe must work through their existing workflows
   (QEMU command line, libvirt, Proxmox, VM managers, private-cloud stacks). A new user-facing CLI is a **no**.
2. **The user flow is: compile QEMU, then use the VM with your own tools, now with a GPU device** (the kf3 device on the QEMU command line).
3. **One patched QEMU binary is acceptable** (it becomes a drop-in for the distro binary). A **QEMU module** (loadable, so stock QEMU plus a
   module) is preferred where it works, and has been a priority from day 1. Until a module works, the patched binary stays.
4. **Hypervisor agnostic** is a goal from day 1 (QEMU first; the core already reaches the hypervisor through traits; other VMMs such as Cloud
   Hypervisor later). *Measured status: only QEMU is supported.*
5. **Bench scripts focus on CI.** The `scripts/bench/*` tooling exists for CI and the release gate, not as the user product. Keep it out of
   the user flow; the user-facing path is documented in `docs/howto/` and uses only QEMU arguments.
6. **Unchecked consequences (for the next work, not decisions):** a tested libvirt XML snippet and Proxmox `args:` example; whether kf3 can be a
   loadable QEMU module or a minimal patch; which host-side pieces a user must also run, so "just a QEMU device" is not overstated.
7. **The broker is display only (owner, 2026-11-15).** The broker is only for those who want a display. Headless VMs (the majority of Proxmox,
   private-cloud and compute use) do not need it; the headless host-side set is the kf3 device, the Rust host crates and the CUDA walker.
   Treat the broker as the replacement for Looking Glass: a VFIO guest has no display in QEMU unless one adds a simple built-in display
   (no GPU acceleration) or a separate daemon such as Looking Glass. Whatever the integration shape (patched binary, module, vfio-user),
   the broker stays an optional, separate piece.

8. **Performance parity goal (owner, 2026-11-15).** The goal is at least 90 percent of native on ALL benchmarks, preferably 95 percent or more,
   across CUDA, LLM, 3D games, graphics design and SPECviewperf (many apps). One benchmark is not parity: the owner recalls about 95 percent on
   Unigine Heaven but only about 60 percent on another; a result like "SPECviewperf overall at 90 percent of native" is worth much more than a
   single Superposition score. *Measured so far (not parity):* Wukong under kf3 about 39 percent of VFIO on the RTX 4070 (36 vs 93 FPS), and
   3DMark Time Spy 7297 with no native reference. A parity claim needs a native or VFIO reference on the same hardware and settings.

## §AR. Release run: standalone parity, requirement vs instruction, manual run first, Windows lanes, 2026-11-15

**STATUS: LIVE, 2026-11-15; owner rulings (stated in conversation, recorded by the coordinator). Whether today's code already meets 2 to 4 is NOT yet audited.**

<!-- src: owner, 2026-11-15, release-gate scoping conversation -->

1. **Windows lanes and proprietary software.**
   - A from-scratch Windows lane (official ISO, installed by the user) cannot run proprietary software, so it is not a release-gate item.
     It is spec-tested only for an app whose vendor ships a portable zip; any other app is excluded.
   - Proprietary apps run only on our prebuilt image, always offline (slirp `restrict=on`).
   - Standalone use stays supported: someone who builds the image themselves can run the same lane. It is not gated.
   - Open probes need a Windows guest anyway. For them a from-spec LTSC image **with internet** is the lane, and it is also where the spec is loaded.
     This keeps a per-lane network switch on the list (online for the LTSC lane, offline for the proprietary image).
2. **Three layers, two formats.**
   - A **requirement** says what is wanted (for example: release gate, tree X, tier Y). CI sends requirements, and so can a manual check.
   - The **Worker** resolves a requirement against its database and writes an **instruction** for a box. Requirement and instruction are
     different formats.
   - The **Python on the box** takes only the instruction: CLI flags for simple cases, a data file for complexity. It runs standalone from a
     file, with a run token from the Worker for reporting.
   - An admin-instructed manual run (an instruction given to a box directly, with no requirement) is a valid extra path.
   - **Instruction sources (owner, 2026-11-15).** A data file is required as an option for standalone use. In an automated Worker run the box
     connects to the Worker with its run token to report and to fetch, so the Worker itself may be the instruction source instead of a file.
     Both sources feed one loader and one validation.
   - *Coordinator refinement (proposal):* the fetch happens in the bootstrap/reporter layer, which already holds the token. It writes the
     instruction to a file, records its hash and hands the file to the payload, which stays tokenless (the payload loses the token, broker URL
     and run id today; `scripts/ci/README.md`). The payload therefore only ever reads a file, and that file's hash is the provenance record.
3. **Anything the Worker can run must be runnable standalone**, in Python, from a CLI: every artifact pinnable from the filesystem; every step
   decomposable (run alone, or from an instruction file identical to what the Worker would give); private R2 configurable.
4. **Coordinator refinements (proposals, not rulings; the owner may overrule):**
   - Do not duplicate the requirement-to-instruction compiler in Python; expose a Worker "resolve only" call if the audit finds none.
   - A manual or hand-edited run counts toward the release gate only if its result records the requirement id, the tree sha, the instruction
     file hash and a valid run token; otherwise it is local evidence.
   - Versioned schemas for both formats; the instruction is validated once at the boundary and carried in a type.
   - A parity test: the Worker-generated instruction for a sample requirement runs in dry-run on the Python side.
5. **Order of work.** The release bar is high on purpose, so: (a) small, independently manageable stages, each with a pass record keyed by the
   exact tree sha (the final full run stays mandatory; `gpu-vast` on master needs the exact final tree); (b) a manual full release run on a
   manual box (the PC when up, else a Vast box) passing **before** any Worker run; (c) Worker "tiny box" runs only when they save time, to
   exercise the waterfall without a full release run, in parallel with (b), never gating it; (d) the real Worker run last.
6. **Rootless proof (owner, 2026-11-15).** The purpose is to prove that kayfabe works in real production as a non-root user with **no
   capabilities** (no `CAP_*`). It is NOT a CI security measure; where it is easy, the release run may add it as a bonus. The run executes
   kayfabe as an unprivileged user that holds no capabilities, with only the device-node access (file permissions or group membership) it is
   granted. Box setup that needs root (packages, device permissions, mounts, driver and module loading) is allowed and kept separate from the
   run. Which steps still need root or a capability is recorded, not assumed (audit: `/root/night/PARITY_AUDIT_20261115.md`, pending). This
   also fits the "unprivileged RM verbs" rule.
   - **Explicit privilege (owner, 2026-11-11, restated 2026-11-15; design in the coordinator's `RELEASE_SEQUENCE.md`, not yet in the repo).**
     The unprivileged account may run privileged commands, but only explicitly: every root step is a greppable `kfroot <cmd>` line.
     `/usr/local/bin/kfroot` runs `sudo -n -E` on a root-owned wrapper that is the ONLY sudoers entry (`ci ALL=(root) NOPASSWD:SETENV`,
     `!secure_path`), so a third-party script that calls plain `sudo X` is refused. The sudo log is the audit: the gate asserts no sudo entry
     other than the wrapper in the kayfabe run window. The kayfabe process tree runs with `NoNewPrivs=1` and `CapEff/CapPrm/CapBnd=0`, so
     `kfroot` cannot be used from inside it. This is accident prevention, not security inside the box: the whole box is an untrusted community
     sandbox and the Worker is the boundary, so no setuid-hardening is required. A small setuid C helper is only a fallback if the image has no
     sudo or env forwarding proves fiddly. Root-owned prerequisites are created by the harness before the drop: `/dev/nvidia*`,
     `nvidia-uvm`, `/dev/dri` nodes (NoNewPrivs blocks `nvidia-modprobe`) and the memlock rlimit.
   - **All tiers (owner, 2026-11-15):** kayfabe's VMs run rootless with group `kvm` in the small, mid and large tiers alike, so a root
     dependency fails on the first PR that adds it. The run records `uid != 0` and the `/proc/self/status` capability and `NoNewPrivs`
     lines in its result; a mutant that runs the step as root must be caught.
7. **Wukong under VFIO passthrough is the most useful reference** for the Wukong TDR (2026-11-15). A pass there makes the kf3 TDR a release
   blocker; a TDR there points away from kayfabe (inferred until runs are in).

## §AQ. A failed rental never means the test failed: the release replaces that leg on the fly, 2026-11-12

**STATUS: LIVE, 2026-11-12; owner requirement, implemented as code and GPU-free tests only (draft PR #362), not deployed.**

Source: the owner's requirement forwarded by the coordinator on 2026-11-12.

<!-- src: owner, 2026-11-12, forwarded requirement "rentals can fail without meaning the test failed" -->

1. **Rentals can fail without meaning the test failed.** If a box rental fails, or a box later becomes unavailable or dies mid-run, the release
   recomputes and replaces that leg on the fly while the other boxes keep running. Only a guest failure is a FAIL.
2. The replacement keeps the die, the CPU-vendor requirement, the slot and price caps and the release cost cap; the re-run allowance is
   `RELEASE_RERUNS_PER_LEG`. Design and tests: `design/V3_RELEASE_RESILIENCE.md`.

## §AP. Whitepaper answers: translation barriers, progress and evidence, 2026-10-28

**STATUS: LIVE, 2026-10-28; owner design clarifications, not hardware qualification.**

Source: the owner's whitepaper answers, forwarded by the coordinator on 2026-10-28 in resume-plan item 2; question numbers refer to the [v3 paper's companion index](whitepaper/kayfabe_v3_index.md).

<!-- src: owner, 2026-10-28, forwarded whitepaper answers and resume-plan item 2 -->

1. **Q1, by design: translation-barrier contract.** Until a refresh, edited leaves may resolve to old backing, new backing or no mapping; unedited valid VAs remain valid throughout.
   After a refresh, edited VAs reflect the state when that refresh started or a later state.
   Refreshes coalesce: the last refresh only has to observe all changes preceding its own start, so refresh spam must not accumulate a backlog of distinct refresh obligations.
   The driver may skip or defer an invalidate on an invalid-to-valid transition for TLB batching within this contract; no exception to the contract is needed.
2. **Q5/Q6, by design: worker parking.** A worker may park only if every pending completion can be serviced through its epoll sources.
   Consuming an eventfd notification must not lose the final wake for outstanding work; waiter registration, rechecking the work condition and the releasing producer's notification must cover release before, during and after registration.
3. **Q7, by design: GPU isolation boundary.** Channels and TSGs are the resource boundary; their privilege and permitted address-space mappings still constrain what work can access, as required by §Q, §AB and §AI.
   This does not settle which cross-client operations need additional ownership checks.
4. **Q8, owner design clarification:** “nesting cost is VFIO-guest only”.
   By design, this clarification supplies no timing data or equivalence between paths.
5. **Q9, by design: correctness oracle.** Require checked output bytes, assertions and cleanup; a process exit status of zero alone is insufficient.
6. **Q13/Q32, owner scope clarification:** support is organised by families rather than a list of individually qualified dies, while per-die facts still follow §A.7 and the provenance requirements in §AF–§AH.
7. **Q23, by design (owner, 2026-10-28):** clocks in this question are measurement instruments, not completion or progress conditions.
8. **Q24, by design:** completion notifications are weak doorbells; a wake prompts inspection of the operation's own completion state and does not identify or prove that operation's completion.

### Code checks accompanying the fold-in

<!-- src: CODE READ, 7dcedefea093de078188efd5f712561bde746362, paths and symbols below -->

**CODE READ at `7dcedefea`:** `crates/kf-mem/src/apply.rs` computes unchanged overlaps in `unchanged_pairs` and subtracts kept intervals before unmapping in `apply_entry`; its `same_mapping` comparison excludes leaf size.
**CODE READ at `7dcedefea`:** `crates/kf-mem/src/sim.rs::apply_and_commit` arms an unchanged-translation guard, `SimRm::check_guard` checks it after host calls, and `check` rejects transients, including declared re-makes.
**CODE READ at `7dcedefea`:** `crates/kf-mem/src/sim/tests.rs` covers a one-page edit within a 16-page row, identical translations across leaf-size changes, and exact partial changes inside a large leaf.
**CODE READ at `7dcedefea`:** `apply.rs::keep_only_what_stays_exact` retains a re-make fallback for a rigid mapping; the tests deliberately plant such an invalid ledger shape, so these source checks are not a universal hardware proof.
**CODE READ at `7dcedefea`:** searching `crates/kf-mem/src` for `sparse`/`SPARSE` finds only the test-only simulator and a test format fixture in `refusal_dump.rs`; `apply.rs::DiffRun` has no dedicated sparse run representation.
**CODE READ at `7dcedefea`:** sparse detection and counting do exist outside that crate in `cuda/walk/kf_walk.cu` (`kf_slot_sparse`, `kf_par_decode_slot`, `kf_par_chunks`), so the search does not establish that the product has no sparse-related code or describe experimental branches.

## A. Standing principles

1. **v3 is a rewrite; never mix planes.** No CPU data plane; no mixing of CPU- and GPU-plane paths.
2. **Host verbs are authored, never forwarded.** kayfabe issues its own unprivileged host RM calls;
   it never replays a guest's privileged control raw.
3. **Completions are host events on an fd — never forged.** ★ (09-26) *A refusal the guest reads as
   "wait over" or "done" IS a forged completion* (`MC_SERVICE_INTERRUPTS`, the sysmembar flush): audit
   non-OK statuses on wait/flush paths first. An entry retired without its work having executed is the
   same defect.
4. **No blocking on a vCPU and none under a lock another thread blocks on.** The one exception: a PRAMIN
   window move = one host map + one mmap, synchronously in the vCPU.
5. **VMM addresses are never guest-chosen or guest-visible.**
6. **§13 unsafe discipline:** no raw VMM pointers in safe code; lengths/offsets validated inside the
   unsafe crates. `kf-cuda` may be a third unsafe crate (09-25).
7. **All GPU families are first-class** (Turing … Blackwell, incl. Hopper). **Derive per die, maintain
   per family:** per-architecture values come from the open guest driver source (ogkm) — generated
   tables, not hand rows; per-die values from unprivileged host controls at VM start.
8. **Compliance principle (09-26):** kayfabe only needs guest *userspace* to work. The guest kernel
   driver is open source, so read what it checks and supply values that satisfy it; prefer deriving
   from source over measuring.
9. **Hostile-guest isolation is the value proposition.** Guest root may only affect isolation *inside*
   the guest, never the host. In-guest isolation matters too: unprivileged guest userspace must not
   reach what the guest kernel protects (09-25 Q7; 09-26 PRIV leaves are withheld from user twins).
10. **Bare metal passes + guest fails ⇒ kayfabe bug.** (09-26) *Always confirm on bare metal first*
    before blaming a box or host.
11. **Don't silently change a constraint — tell the owner.** For open design choices, don't wait:
    run the best-bet **experiment on a sub-branch** (never master), report evidence, let the owner decide
    what lands (09-26).

## B. Specific decisions (2026-09-25)

- **Q7 kernel identity** = RM's `internalFlags` PRIVILEGE stamp (+ the RM-internal handle range).
  Guest-root (ADMIN) channels stay Passthrough.
- **Translated** rings/pushbuffer/USERD live outside the GPGA but inside the VA space; operands still
  reference the GPGA and are never copied.
- **`GPU_PROMOTE_CTX`: stub** — the host twin already holds the real context; golden context is
  guest-kernel-only.
- **Q8 memory plane:** the GPU walker sends only a **diff** against its last snapshot, kept **in vidmem**;
  no checksums; the host kernel is the ledger; **commit-on-ack** (only host-confirmed placements commit).
- **Non-GSP guests:** a future target, not v1 (Windows can force GSP on).
- **Q11:** narrow the ring-collision refusal to the offending leaf — self-harm only, never inducible
  across isolation by unprivileged guest userspace.
- **Per-twin RC** is fine iff unobservable to guest userspace and leaving no state the guest RM doesn't
  expect.
- **Hopper stays supported** (e.g. MEM_OP_D MMU_OPERATION is Hopper's invalidate: serve it, don't refuse).
- **Suite budget** may be raised when measured variance justifies it (it is 180 s since 09-26).

## C. Roadmap (2026-09-26), in order

1. Apps working (the nvkvm-pv CUDA app set; works/fails, not parity).
2. All headless-graphics tests nvkvm-pv passed.
3. Display (scanout), then display apps / a desktop (Mint) that nvkvm-pv ran.
4. In parallel: doorbell performance — non-nested baseline and host-side exits first, optional
   Linux guest helper afterward (refined 2026-09-28, §D); Blackwell (done: RTX 5080 30/30).
5. Driver matrix — the same driver range nvkvm-pv supports (535 → 610), both driver axes.
   ★ **Refined 2026-09-28 (owner): keep the GPU-architecture axis — the sweep covers every family
   Turing and newer** (host driver × guest driver × GPU arch), not one bench die. Vast VM offers seen on
   2026-09-28: TU116 (GTX 1660 S/Ti) and TU106 (RTX 2060 S), GA10x, AD10x and GB20x in number; **no
   GA100, GH100 or GB10x as VMs** — those rows stay source-derived until such a host is available.
6. Windows guest last. ★ **Refined 2026-10-01 (owner):** before Windows, the display must work with a
   **stock guest and no guest-side tweaks**, and frames should leave through a **VMM-agnostic broker**
   rather than QEMU's SDL/GTK UI (`design/V3_DISPLAY.md`, the NEXT note at the top).
   ★ **Demo milestone (owner, 2026-10-01):** a **game running in a Windows guest**, plus **SolidWorks**,
   shown in a window on the **Linux host desktop** (through the broker) — then a new r/VFIO post. The
   point to show: one GPU shared with the host (no VFIO passthrough, host desktop stays alive), the stock
   NVIDIA driver in the guest, hostile-guest isolation. Pick a game whose anti-cheat tolerates VMs.
- ⊘ *Superseded 2026-09-28 by the refinement of item 5 (Turing is on the arch axis, so it runs on
  hardware before the sweep):* Later, if time: **Turing** on hardware (its GSP model is source-derived only).
- **All working, verified work lands on master.** Nothing verified may be stranded on a branch.

## D. Guest doorbell helper module (2026-09-26)

- **Approved for design; security posture accepted:** giving guest *root* write access to the real host
  doorbell page (it can ring any host token) is accepted risk, as in shared CUDA containers — a ring only
  makes the GPU re-read that channel's own GP_PUT. Stock guests keep the trapped path.
- Design specifics (table = one atomic u64 per guest token; always opportunistic; virtio discovery;
  inert on bare metal; BAR1 map immediate / unmap waits for the module's ack; free load/unload order;
  BAR0 offset supplied per family; multi-GPU per BDF): `docs/design/V3_GUEST_DOORBELL_MODULE.md`.
- **Windows:** a legitimate Windows equivalent is not possible today (WDDM rings from the kernel
  driver); possibly not needed — measure doorbells/token on a Windows guest first
  (`docs/design/V3_WINDOWS_DOORBELL_RESEARCH.md`).
- **DECIDED 2026-09-28: baseline first, then the helper.** Measure on non-nested hardware and
  improve host-side exit handling first; **ioeventfd remains in the pipeline**. An optional
  paravirtual interface in a modified guest NVIDIA driver stays open for later, not selected now.
  Measure vCPU return separately from the eventual GPU doorbell: asynchronous dispatch can help
  a deep queue while hurting an idle/synchronous launch. Coalescing must preserve progress and
  ordering. The owner's ~70% non-nested throughput expectation and possible Windows batching
  advantage are hypotheses, not measured results. A Windows kernel transition is not itself a
  hardware VM exit; collect submission/doorbell counts before comparing OSes.
- **Refined 2026-09-28: no deliberate coalescing delay.** Ring as soon as the asynchronous worker
  can act. No batching timers or extra waiting to accumulate kicks; CUDA already batches work.
  Coalescing is only a benign side effect when several notifications are already pending, not a
  performance objective in itself. Notification counts need not match host MMIO ring counts;
  eventual GPU notification or translated/emulated queue inspection must never be lost.
- **BAR1 doorbell (Hopper+):** must follow where RM places it — built (`V3_BAR1_DOORBELL.md`).
- **Refined 2026-09-30 (owner, in the session):**
  - *ioeventfd is for every doorbell, not only passthrough:* "doorbells always need to be forwarded as
    quickly as possible", so the non-blocking register drainer is the right place; it rings the host twin
    inline for Passthrough and hands Translated/Emulated tokens on. (This is what `v3-mc22` built:
    `design/V3_DOORBELL_IOEVENTFD.md`; still default **off** pending a non-nested measurement.)
  - *Nested is a real deployment, not an edge case* ("think microVM containers"). The owner expects the
    guest helper module to bring a nested doorbell to about **1–2 µs**, which they consider sufficient
    for parity. ⚠ Hypothesis: the helper is still design-only and unmeasured.
  - *Why the C reached host parity despite trapping every doorbell:* its 1.05× (llama.cpp 49.9 vs
    47.5 tok/s) was measured on **bare metal**, an RTX 3050 kiosk PC (`docs/archive/STATUS_DETAIL_pre_v3.md`
    "Hardware"), not on a nested Vast box. ★ (owner, 2026-09-30) That C is **nvkvm Mode 2 — the same shape
    as kayfabe** (stock guest driver, emulated GPU + fake GSP, trapped doorbells), **not nvkvm-pv**; that is
    why it is the relevant baseline. Its copy-engine work ran on the CPU, so it speaks to the doorbell cost,
    not the copy path. On bare metal a KVM exit costs a few µs, not the ~15–50 µs of
    a nested box, so ioeventfd alone may reach parity there. Bare metal is also the most useful case for
    Windows guests. ⚠ Hypothesis until a non-nested host is measured (§D "baseline first").

## E. UVM demand paging (2026-09-26)

- Unprivileged-only is believed impossible (host replayable faults need a host-UVM-owned VA space; the
  fault buffer is kernel-privileged; the GPU reaches memory at the guest's VA).
- **A privileged host piece is allowed, for UVM only, and it must not trust the VMM**; the rest stays
  unprivileged and untrusted.
- **DECIDED 2026-09-28: full host CUDA must coexist.** A narrowly scoped patch to the host's open
  **nvidia-uvm (b3)** for guest UVM fault handling is acceptable and is the selected route. Keep
  ordinary host CUDA behavior, the CUDA-based walker, and non-opted-in address spaces working.
  **Supersedes the 09-26 preference** for a separate replacement module: N4 takeover prevents
  stock UVM coexistence in the same host kernel (the 580.159.04 callback registration is global,
  not per GPU). N4 remains historical research, not the next implementation experiment.
- **RULED 2026-10-02 (owner) on the `v3-uvm-guest` questions — both NO:**
  - *No MMIO/BAR CPU reads of guest page tables* (the branch's §3.8a experiment reads them through
    CPU views of the store). Prefer the GPU walker; if a CPU decode is unavoidable on the fault path,
    the words must come from a **copy-engine snapshot** into host memory, not MMIO reads.
  - *A ~4 s stall of every tenant's GR work per parked fault is not acceptable.* Cause, measured on
    the RTX 3060: a GR context stalled on a parked replayable fault cannot be context-switched out, so
    no other GR work runs (other VMs, host CUDA, kf3's own walker) until the fault is replayed; the
    bound is the host's ~4.3 s ctxsw watchdog (Xid 109), not a kayfabe timeout. ⇒ The fault path must
    be GR-free end to end and fast, the park time bounded far below the watchdog, and UVM is **not
    merged** until the per-fault GR hold time is measured and acceptable (two-VM test, one hostile).
- Stock UVM read-duplication (`cudaMemAdviseSetReadMostly`) is not a kayfabe mode — kayfabe must *handle*
  it correctly (read-only PTEs, collapse on write): permission bits are now carried.
- **Next: bounded b3 host-only proof before guest integration.** Require real fault delivery,
  repair/replay with correct data, scoped cancellation, ownership-negative tests, bounded teardown,
  and concurrent ordinary host CUDA. Existing `DupAddressSpace` / `RetainChannel` helpers do not
  provide caller ownership authentication for free; the new boundary must enforce it. No production
  support or coexistence result is implied by this direction. Historical research continues through
  `v3-uvm-e6pp` at `c6765f5c`; its phase 0 did not prove replay with a replacement module present.

## F. Work practice (2026-09-26)

- **Agents:** each on its own worktree + sub-branch and its own vast box(es); builds on the box; local
  cargo only under a global flock with 2 jobs and a deleted-after target dir (the dev host is shared).
- **Boxes:** untrusted, not guaranteed to persist, no secrets, no executables copied back, evidence
  pushed to git after each run, keep only boxes in use, teardown only by ids you created
  (`scripts/bench/box/README.md`).
- **Security-policy changes need explicit owner review before merge.** **APPROVED 2026-09-28:**
  the 535/545 capability extension (`a50265f8`, formerly `ee35ca4a`, on `v3-drivers`). Independently
  audit the shared groups omitted by the header sweep, compare complete resolved existing 550+
  policies (not just counts), and pass the required exact-revision tests before promotion. This
  approval does not claim end-to-end 535/545 application support or relax other policy rules.
- **Refined 2026-09-30 (owner):** *"a chat is no durable artifact, neither this host — keep GitHub as
  backup for all valuable work."* The dev host is not durable storage either: push branches (a
  `backup/<host>-<date>/` namespace for unreviewed local work, after a secret scan) and record the resume
  point in `docs/STATUS_AND_HANDOFF.md` §0 before any pause.
- **Pacing (owner, 2026-09-30):** the owner expected check-ins before long unattended stretches ("thought
  you would stop earlier and ask") while welcoming the results; weekly-usage aim ≈ 14 % per day. Check in
  before launching a new wave of agents or boxes; wrap up (push, stop notes, destroy boxes) when asked.
- **Storage and execution reaffirmed 2026-09-28:** Vast is untrusted, replaceable compute, never the
  only copy of unique work. Commit/push source and useful text evidence from trusted local storage;
  do not send account credentials or forward the SSH agent. The owner authorized taking over all
  instances in the recovery inventory and retiring those no longer needed, after preserving work.
  **Superseded 2026-09-29:** the owner authorized retiring the Paguro Windows box after saving
  any unique work; retention is no longer required. The /dev/sdb SSD is
  spare workspace; regenerate/download caches, builds and VM images rather than lose unique work.
- **Public repository: which addresses may be committed (owner, 2026-10-07, relayed by the
  coordinator).** Private LAN addresses, Vast instance ids and Vast IP addresses may appear in commits.
  The owner's home public IP and any Scaleway public IP of the controller must NEVER be committed.
  Scan every diff for non-private IP addresses before pushing.

## G. Licence (2026-10-02)

- **Ruling:** dual-license the whole repository as **`Apache-2.0 OR GPL-2.0-or-later`**. The owner's
  words: *"Same as ogkm. We satisfy both, user may chose either"*. The owner confirmed the exact
  expression on 2026-10-02.
- **Done on branch `v3-dual-license`:**
  - `LICENSE` states the grant and its exceptions. `LICENSE-APACHE` and `LICENSE-GPL` hold the full
    texts.
  - The `Cargo.toml` `license` fields are updated: the workspace and both detached `gen` crates.
- **Exceptions, as `LICENSE` lists them:**
  - the `third_party/` submodules;
  - files derived from gVisor's nvproxy, which stay Apache-2.0 only;
  - files that carry their own notice;
  - third-party data inside recorded traces.
- **Still open, under task I7 (`docs/design/V3_SWEEP_AND_INSTALL.md`):**
  - Re-derive the nvproxy-derived allowlists (`kf-abi`/`kayfabe-abi` `capability.rs`) from ogkm
    headers. `kf-qemu` links `kf-abi`, so until then a QEMU binary built from this tree is not
    distributable under the GPL.
  - Per-file SPDX headers, starting with the three kf3 overlay files.
  - `LICENSES/` in the release tarball.
- **For the owner to decide:** whether the archived C reference traces stay in the public tree.
  - `archive/nvkvm/traces/mode2_c_reference/` records PROM reads as served values.
  - The C emulator served those reads from a real GA106 VBIOS dump: its `vbios=` property; md5 in
    that directory's README.
  - The grant does not cover that data (`LICENSE`, exception 4).

## H. Feature scope after a working product (2026-10-02)

- **Stub what workloads do not need** (owner). Stub a refusal when it only blocks a feature
  workloads do not need, especially one an unprivileged host process cannot issue.
  - Named: debugger, profiler, NVLink, ECC.
  - Confidential compute is impossible. Its attestation is rooted in the physical GPU's keys and is
    never faked.
  - A stub answers the way a GPU without the feature answers: capability absent, or not permitted.
    It never answers `OK` to an action; that is the forged-completion trap of 2026-09-26.
    ⊘ *Refined 2026-10-07 by §S:* a privileged host-management action with no compute or
    display effect on the guest (RC-recovery policy, notifier arming for thermal or power
    events, for example) may be accepted silently, with a coherent "feature absent" state.
    GPU work that reached hardware is still never forged.
  - Read in ogkm-580 the same day:
    - The **debugger** is open to unprivileged users for their own processes
      (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_sm_debugger_session.c:270-301`). kayfabe
      could forward it later.
    - The **profiler** needs root or `CAP_PERFMON` unless the host admin sets
      `RestrictProfilingToAdminUsers=0`. The default is 1 (`ogkm-580: kernel-open/nvidia/nv-reg.h:965`;
      `ogkm-580: src/nvidia/src/kernel/gpu/hwpm/profiler_v2/kern_profiler_v2.c:31-63`), so a stub
      matches what a non-root user gets on bare metal. If it is ever offered, make it opt-in for
      single-tenant hosts only: profiler counters are a side channel across tenants.
    - The debugger does **not** service GPU faults. cuda-gdb's own table lists the memory
      exceptions as "Not precise" and says continuing after them "can lead to undefined behavior"
      (NVIDIA's cuda-gdb documentation, read 2026-10-02). Resumable fault service exists only below
      the debugger, in UVM, and in kayfabe's EFS patch (`tools/uvm_efs/patch/uvm_efs.c`).
- **Guest-side NVIDIA vGPU: crossed off** (owner).
  - Running NVIDIA's licensed vGPU guest stack on kayfabe needs its license check faked, the way
    vgpu_unlock setups do. That is piracy, however little reverse engineering it takes.
  - A user who holds a vGPU license should run NVIDIA vGPU and does not need kayfabe.
- **Ideas for later, not rulings.** The full plan, with the stock tier and the opt-in stages, is in
  `design/V3_COOPERATIVE_TIERS.md` (2026-10-03).
  - Host-side vGPU-style sharing, one device per VM, through Linux mediated devices, so Proxmox,
    libvirt and OpenStack attach kayfabe unmodified. Not NVIDIA's vGPU software, and no licensing.
    See `design/V3_VFIO_USER_FRONTEND.md` §2, option 0.
  - MIG: map host MIG instances into guests, on MIG-capable cards.

## I. Release scope and priorities (2026-10-03)

- **Post-release:** the cooperation stages in `design/V3_COOPERATIVE_TIERS.md` (owner). These are
  the patched guest driver, the patched host driver and the host helper module.
- **Release focus, in order (owner):**
  1. the display path (`design/V3_DISPLAY.md`, NEXT block of 2026-10-03);
  2. the install path and the sweep (`design/V3_SWEEP_AND_INSTALL.md`);
  3. Windows.
- **Non-GSP (pre-Turing) was raised as possibly more useful.** The review's view:
  - It is the largest new emulation surface: the GSP-side three quarters of RM, running against
    emulated registers.
  - It targets a frozen platform. R580 is NVIDIA's last driver branch for Maxwell, Pascal and
    Volta, and CUDA 13 dropped them.
  - §B ("Non-GSP guests: a future target, not v1") stands. Decide after Windows, on demand.
- **Guest doorbell helper (stage 1): a release candidate,** decided after two steps:
  - a per-token breakdown on nested boxes, where doorbells explain about 22 of PyTorch eager's
    ~58 ms per-token gap (0.29× of host) and the rest is unattributed;
  - a non-nested baseline (§D, 2026-09-28).
  - llama.cpp already decodes at 0.92× of host there.
- **Managed memory (UVM demand paging): not a release target (owner).** PyTorch, TensorFlow, JAX,
  vLLM, llama.cpp by default, games and Vulkan, GL and desktop apps do not use it.
  - Exceptions to document:
    - opt-in switches such as llama.cpp's `GGML_CUDA_ENABLE_UNIFIED_MEMORY`;
    - RAPIDS cudf.pandas, which, as far as the review knows, defaults to managed memory when the
      GPU reports concurrent managed access.
  - **Release item: unsupported must fail loudly.** The four managed-memory apps fail on master,
    and `conjugateGradientUM` prints a wrong answer with `result = SUCCESS`
    (`design/V3_APP_MATRIX.md`). Every such fault must reach the app as an error.
  - A stock Linux guest cannot use the Windows-style mode: guest UVM hard-codes fault support
    (`ogkm-580: kernel-open/nvidia-uvm/uvm_ampere.c:81`). Windows guests should get managed memory
    without a fault plane; that is unverified until a Windows guest runs CUDA.
- **Pinned DMA stays supported.** `bandwidthTest` and `simpleZeroCopy` pass on master
  (`design/V3_APP_MATRIX.md`).
  - Coverage gap: no row uses the CUDA virtual memory API (`cuMemCreate`/`cuMemMap`), which PyTorch's
    expandable segments and vLLM rely on. Add a sample such as `vectorAddMMAP` to the sweep.

## J. Kernel-module flavours (2026-10-03)

- **Closed (proprietary) host modules are supported, never refused (owner).** nvkvm-pv forwards the
  same RM ioctls on both flavours and met few compatibility differences between them. kayfabe's host
  side uses only that unprivileged ioctl API and nvidia-uvm, whose source ships in both packages.
  - Preflight reports the flavour and does not refuse it. Until a sweep covers a closed-host cell,
    `SUPPORT.md` shows it as "accepted, untested".
  - NVIDIA's own limit: the closed module cannot drive Blackwell (`nvkvm-pv:docs/howto/sweep.md:303`),
    so closed-host cells exist for Turing, Ampere and Ada.
  - This supersedes the review's recommendation of 2026-10-03 to refuse closed hosts
    (`OWNER_QUESTIONS_2026-10-03.md`, Q5).
- **Closed guest modules are a sweep axis, and Windows makes them a must (owner).** Windows has only
  NVIDIA's proprietary driver.
  - At 580 both flavours use the GSP firmware by default on every Turing-and-later GPU (`README.txt`
    of `NVIDIA-Linux-x86_64-580.159.04.run`, chapter 44C, read 2026-10-03). A closed Linux guest
    therefore reaches kayfabe's fake GSP with no module option. Older branches are to be checked per
    tag.
  - The open question is whether the closed guest's kernel RM issues RPCs or controls the open one
    does not. One closed-guest cell per family in the band tier answers it.
  - As far as the review knows, the closed Linux RM and the Windows RM come from the same NVIDIA
    sources, so the closed-guest cells are also the earliest Linux-side signal for Windows
    (unverified).
- Answers `design/V3_SWEEP_AND_INSTALL.md` §4 Q5.

## K. The boot display's option ROM is one embedded blob plus generated config (2026-10-03)

- **Owner:** *"generate the uefi data in kayfabe and give it as blob in the rom. So there is no rom
  per gpu or similar, all is given as config data, just like cuda."*
- The GOP driver is one constant `.efi`, embedded in kayfabe with `include_bytes!`. It is not a
  separate firmware file, there is no `romfile=`, and users install nothing per GPU.
- **No compiled binary is committed** (owner, the same day: *"We aren't going to put compiled stuff
  in the repo right? … the efi driver is compiled when building the repo."*). build.rs compiles
  `firmware/kf-gop` during the normal cargo build, with the nested-cargo pattern that already embeds
  the musl isolate (`crates/kayfabe-isolate-host/build.rs`). `rust-toolchain.toml` lists
  `x86_64-unknown-uefi` beside the musl target, so rustup installs it wherever the repo builds. The
  PTX kernels the owner compared it to are hand-written source, not build output
  (`crates/kf-cuda/src/display.rs:21`).
- kf3 generates everything per device when it starts: the PCI ROM header and PCIR (the identity kf3
  already presents) and the `KFGP` config blob (BAR, offset, size, mode, pitch, format, EDID). It
  wraps the constant `.efi` with them in memory and serves the result as its ROM BAR.
- The driver is byte-identical on every host, so one future Secure Boot signature covers all of
  them.
- Design: `traces/v3_design_review_20261003/` (gop), and `design/V3_DISPLAY.md` once `v3-gop-rom` and
  `v3-gop-kf3` land.
- **aarch64 later (owner, the same day: *"Also later needs aarch64 as well."*).** x86_64 ships first,
  but the ROM stays arch-neutral:
  - the ROM header's EFI machine type comes from the built driver (0x8664 or 0xAA64), never a
    constant;
  - x86 port I/O, used for test-build debug output only, is behind `cfg(target_arch = "x86_64")`;
  - build.rs builds the driver for the arch kayfabe itself is built for;
  - CI's aarch64 job also builds the driver for `aarch64-unknown-uefi`, so x86-isms cannot creep in.
  - Unchecked: whether AAVMF (OVMF's Arm build) runs PCI option ROMs.
  - ⊘ **CORRECTED the same day — the note below overstated it; there is no regression against bare
    metal.** nvidia.ko itself maps the framebuffer BAR as **Device-nGnRE** on arm64 when write
    combining is asked for (`ogkm-580: kernel-open/common/inc/nv-pgprot.h:76-80`, used for
    `NV_MEMORY_TYPE_FRAMEBUFFER` by `kernel-open/nvidia/nv-mmap.c:355-363`, `:587-597`; WC is allowed
    on aarch64, `nv-linux.h:307-308`). So the host's own CUDA gets Device memory for BAR1 too. KVM's
    Device stage-2 for these views equals what bare metal uses, and the guest's own nvidia.ko asks for
    the same type. What remains:
    - code that maps the boot framebuffer expecting Normal-NC must use aligned accesses (the GOP
      driver's `Blt`: aligned volatile stores, no `DC ZVA`-style memset; the guest kernel's I/O
      accessors already align);
    - the `VM_ALLOW_ANY_UNCACHED` patch below would give *more* than bare metal (Normal-NC), which
      NVIDIA chose not to use on arm64, and ARM does not guarantee Normal-NC is safe on every MMIO
      region (`drivers/vfio/pci/vfio_pci_core.c:1815-1830`). Not planned.
    - The same holds for BAR2 (PCI BAR3) and the PRAMIN window. Under the single store
      (`design/THE_CONSTRAINTS.md:1011-1020`) every mapped page of all three windows is a view of
      the one reserved host-VRAM object (`crates/kf-qemu/src/mem.rs:1-35`). Only unmapped pages show
      the per-window scratch memfd, which exists because a memslot hole kills the guest. Since
      2026-10-03 that scratch is one small tile per window, repeated, so a guest touching every
      unmapped page costs the host at most one tile per window (`design/V3_P4_PORT_MAP.md` Q3).
  - (superseded, kept as written) ⚠ **Found 2026-10-03, and it concerns all of kf3 on arm64, not only the ROM.** arm64 KVM maps a
    non-cacheable PFNMAP memslot as Normal-NC only when the host VMA carries `VM_ALLOW_ANY_UNCACHED`,
    and as Device memory otherwise (Linux 7.1 `arch/arm64/kvm/mmu.c:1966-1968`). vfio-pci sets that
    flag (`drivers/vfio/pci/vfio_pci_core.c:1831`); nvidia.ko never does (no occurrence in
    `ogkm-580: kernel-open`). kf3's BAR1 views are nvidia.ko mmaps, so on an arm64 host the guest
    would see the framebuffer and every BAR1 view as Device memory: no write combining, and an
    unaligned access faults. The likely fix is the one-flag host patch in nvidia.ko's mmap path,
    since a host patch is already required for UVM (`design/V3_COOPERATIVE_TIERS.md`).

- **Secure Boot (owner, 2026-10-04, verbatim):** *"In qemu we can just enable 'secure boot' for a
  windows vm and self sign the rom, I mean windows vms just work normally without complaint"*.
  - The ROM's EFI driver is signed with a kayfabe key, and the VM's OVMF variables enroll
    Microsoft's standard keys plus that certificate in `db`. Windows' boot manager and the ROM then
    both verify, and the guest runs with Secure Boot on.
  - The private key is never committed (the repo is public). It is generated per installation or
    per build on the user's machine, and that installation's variables file enrolls its
    certificate.
  - A guest that seals BitLocker to TPM measurements may ask for its recovery key once after the
    ROM changes, because the measurement of the option ROM changes.
- **The vTPM (swtpm), from the owner's question the same day** (*"For swtpm a secure seed must be
  provided probably?"*):
  - ⊘ *Corrected the same day; the first wording ("no seed is supplied") was too loose.* Owner,
    verbatim: *"Yes but tpm should persist reboot though. And a hash of non secret values isn't
    secure. So some secure seed must be stored right."* Right.
  - **A secret seed is stored: the state file is the secret.** `swtpm_setup --tpm2` manufactures the
    TPM once per VM. The primary seeds (endorsement, storage, platform) are random secrets from the
    host's CSPRNG, never derived from non-secret values such as the VM's name or UUID. swtpm keeps
    them in its state file. Everything the guest seals to the TPM, BitLocker and Windows Hello
    included, depends on them.
  - **It persists.** The same state is reused on every guest boot, every QEMU restart (kayfabe
    restarts QEMU on each guest reboot) and every kayfabe update. Manufacture happens once, and the
    tooling refuses to re-run it on an existing state. Losing or regenerating the state is like
    replacing the TPM: BitLocker asks for its recovery key. The VM's OVMF variables file is per-VM
    persistent state too.
  - **It is protected like a key:** one state per VM; owner-only permissions; kept with the VM and
    backed up with its disk, as a secret; never copied into an image, a template or another VM
    (a copy duplicates the seeds and the endorsement key); never committed. If it is encrypted at
    rest (`--key`/`--pwdfile`), that key is a random host-held secret.
  - A self-signed EK certificate is enough for Windows 11 and BitLocker.
- **Bench Windows guests run without BitLocker** (owner, 2026-10-04, verbatim: *"Also disable bitlocker
  in the windows guest, its useless for our vm."*). The unattended install prevents BitLocker and
  Windows' automatic device encryption from the first boot, and the lane checks the volume is fully
  decrypted with protection off, after install and again after the NVIDIA driver install. This is a
  bench setting: a user's own Windows guest may still use BitLocker, so the TPM-persistence rules
  above still apply to it.

## L. Broker frames: a GPU copy into kayfabe's own frames, never guest memory (2026-10-03)

- **Owner:** *"exact zero copy isn't needed though, what we do need is that we can avoid a GPU-CPU copy.
  So if you need an object to export to the screen that contains a frame you can also copy directly.
  Is maybe better for security because a shared RM object lets the guest change the bytes
  underneath."*
- No guest surface and no slice of the store is ever exported to the broker or any other process.
- For a compositor on the same NVIDIA GPU, kf-disp copies each finished frame GPU→GPU into a frame
  object kayfabe allocated itself in host VRAM. That object is exported as a dma-buf with a format and
  modifier the compositor imports. The frame never crosses to the CPU, and the guest cannot change it
  after the copy.
- The existing rungs stay as fallbacks: a linear dma-buf copy in host RAM for compositors on another GPU
  or vendor, and F_SHM.
- Design pass of 2026-10-03; research in `traces/` once the broker branch lands.

## M. The display frame-rate bound is configurable (2026-10-03)

- **Owner:** *"I think the fps bound must be configurable."* Context: with `GF100_DISP_SW` twinned on
  the host (`x11-dispsw`, still an experiment), host RM fires vblank releases at the host head's vblank,
  and immediately on a headless host. Vsync'd X11 clients would then run uncapped.
- One device property (working name `display-max-fps`), defaulting to the virtual monitor's refresh.
  - **KMS flips** (Wayland, fullscreen X, nvidia-drm) are paced by kf-disp's own vblank timer, so the
    bound is exact there. The EDID's preferred mode follows the same rate.
  - **X11 display-SW paths** (windowed GLX, X11 Vulkan FIFO): ⊘ corrected after the required
    D4 hardware run, 2026-10-04. On candidate 2 `9d82f259`, NVIDIA 580.159.04 on GA106,
    these clients follow the guest driver's consumption of kayfabe's emulated vblank. Cap 30
    gives GLX about 30 FPS (also with a forced 60 Hz raster) and actual Vulkan FIFO presents
    at 29.976857 FPS; the IMMEDIATE control reaches 1557.8 FPS. Thus the clamp bounds the
    measured vsync paths. The earlier proposal for extra notify/submission pacing levers is
    superseded for these paths; this is not a promise to throttle non-vsync rendering.
    Evidence: `traces/v3_candidates/cand2_20261004/{x1130,present_timing}/`.
  - The status line reports the achieved rate per path, so a bound that does not hold is visible.

- **Design decisions (2026-10-04).** The design is reviewed and kept outside the repo until the
  `v3-maxfps` branch carries it. Owner, verbatim: *"Why copying when not flipping. Yes a tearing copy
  seems not great though. The rest seems good to md"*.
  - **Mechanism:** the limit clamps kf-disp's own emulated vblank tick (the display thread's
    deadline). Nothing blocks a vCPU, and no lock is held while waiting.
  - ⊘ **D1 SUPERSEDED 2026-10-17 (§AK, below; the owner's decision (a)).** The guest's flips are
    NEVER parked to bound the display rate: an immediate flip latches and completes at once, and the
    rate is bounded by the COPIES (the newest latched frame, at most once per tick). `[measured,
    Windows 11, Unigine Heaven D3D11, build 134dcb54, 2026-10-17]` the gate parked the guest's present
    loop in exclusive fullscreen (windowed ~200 fps, fullscreen ~30 fps; design in
    `docs/design/V3_DISPLAY_PACING_20261017.md`). The paragraph below is the superseded text, kept as
    history; `Engine::tear_gate` and `KF3_TEAR_GATE` no longer exist.
  - **D1, tearing flips are gated (adopted) — SUPERSEDED, see above.** An async/tearing flip that arrives sooner than the
    limit allows waits for the next tick. In kayfabe a flip copies a finished buffer, so the gate is
    about rate, not image tearing.
  - **D2, copies made without a flip (proposed by Claude in answer to the owner's question; adopted
    unless the owner objects).** These exist for front-buffer rendering: the boot console, X11
    without a compositor, front-buffer applications. kayfabe has no physical scanout, so without a
    copy the host never sees those writes.
    - Instead of a fixed 30 Hz timer, a copy is made at the head's emulated (clamped) vblank. That is
      real scanout's cadence and phase, so tearing is no worse than bare metal.
    - A copy is sent only when a GPU-side checksum of the surface changed.
    - Nothing is copied while nobody watches. A `screendump` asks for a fresh copy on demand.
  - **D3 (adopted):** values above 75 Hz are refused. The virtual monitor is single-link DVI
    (165 MHz), so 1080p tops out near 71 Hz.
  - **D4 (adopted, verified 2026-10-04):** the required `display-max-fps=30`, `x11-dispsw=on`
    run supports pacing through the guest's vblank consumer. The correction is folded above;
    §N's separate conditions for enabling x11-dispsw by default remain outstanding.
  - **D5 (adopted):** unset means a cap of 75 Hz, and the EDID stays byte-identical to today.

## N. X11 desktops: GF100_DISP_SW option A is the design (2026-10-03)

- **Owner, after the design was laid out:** *"I think this is the best design i intended."* This answers
  `OWNER_QUESTIONS_2026-10-03.md` item 2.
- **The design:**
  - A guest `GF100_DISP_SW` (`0x9072`) allocation gets a real host twin under its channel's host
    twin. Its parameters are authored by kayfabe (head 0, displayMask 0, caps 0); the guest's
    parameters are never forwarded.
  - The guest's display-SW software methods trap on the host GPU to host RM. Host RM writes the
    vblank release directly into the guest's semaphore memory. That memory is a guest-RAM page or a
    store slice, mapped with a host kernel mapping (`NVOS46_FLAGS_KERNEL_MAPPING_ENABLE`) for exactly
    that memory. kayfabe acts only at allocation, mapping and teardown, not per vblank.
  - Pacing follows §M.
- **It stays inside the single store (owner, the same day: "the guest cannot get an object outside
  its allocated vram").** The object is not memory the guest can address, and releases land only
  in the guest's own mappings. The host client that the address check uses must hold only the
  guest's store, guest RAM and twins; kayfabe's own allocations, such as the broker frame slots,
  live in separate clients. This is pending the client audit of 2026-10-03.
- **Default-on once all of these hold, on hosts whose GPU has a display engine** (refusal by name
  stays the fallback elsewhere):
  - a box run shows releases landing: Cinnamon X11 up, X11 vkcube presenting, vsync clients
    advancing;
  - per-VM caps on display-SW objects and kernel-mapped bytes exist, and refuse by name;
  - the client split is built;
  - a release aimed outside the guest's own memory lands nowhere outside the store and guest RAM.

## O. The guest cursor (2026-10-03)

- **Owner:** *"For hover broker receives the cursor image of the guest, and sets using wl/x api that as
  cursor … if the guest doesn't draw a cursor then it remains hidden in hover/absolute. For grab mode …
  we follow the cursor as is."* And: *"frame copy in gpu is cheap. Best is copy to a nvidia buf on the
  gpu … draw cursor there, then copy to linear over dma as dmabuf (in case of host ram)."*
- The host's hardware cursor plane is never used. The guest programs kf-disp's emulated cursor channel,
  and kf-disp knows the cursor's image (from the store), hot spot, position and visibility per head.
- **Hover / absolute pointer:**
  - The broker receives the guest's cursor image and hot spot, and sets it as the host cursor
    (Wayland `wl_pointer.set_cursor`; X11 an ARGB cursor on the window).
  - The guest's cursor position is ignored, because the absolute device already makes the host
    pointer the guest pointer. A cursor move produces no frame.
  - If the guest hides its cursor, the host cursor is hidden.
  - The image is sent only when it changes, detected by a hash.
  - kf-disp does not compose the cursor into frames in this mode, so there is never a second,
    lagging cursor.
  - If the broker scales the guest frame, it scales the cursor image and hot spot by the same factor.
- **Grab / relative pointer:** the guest owns the position. kf-disp composes the cursor as the top layer
  of the frame (display step 3d), and frames are coalesced to the display rate (§M).
- **XOR / monochrome cursors** cannot be expressed as a host ARGB cursor, so they are composed into the
  frame in either mode. The compose kernel gains an XOR blend.
- **Where composition happens:** always on the GPU, into kayfabe's VRAM staging frame. The GPU-copy rung
  packs that frame into the exported VRAM slot (§L). The host-RAM rungs (linear dma-buf, F_SHM) take
  one copy-engine DMA from staging into the CUDA-registered host frame; the CPU never copies a frame.
- **Protocol:** the broker protocol gains a cursor message (image fd, size, hot spot, hide) behind a
  capability bit. A broker without the bit gets composed cursors. The change goes into nvkvm-pv's
  broker too, so both projects get it.

## P. A security audit before the first public binary (2026-10-03)

- **Owner:** *"Maybe worth doing a security audit. Meanwhile continue on display."* The owner named ten
  areas: no host pointers in safe code; unsafe code bounds-checks every input; races and
  interleavings, with formal checkers where useful; guest DoS points; the security model by role;
  every RM call and ioctl we make; the PTX page-table walker against hostile tables; integer, cast
  and compiled-out checks; prove what can be proved and record what is uncertain; and bug classes
  from history.
- **Plan and timing:** `design/V3_SECURITY_AUDIT_PLAN.md`. Stage 1 (read-only inventory areas) runs
  now, alongside the display work. Stage 2 runs after the display and security branches merge.
- **Gate:** no public binary ships before every stage-2 blocker is closed or accepted by the owner.
- **Out of scope:** sandboxing the VMM process. The owner, the same day: *"Sandboxing vmm is not our
  job."*
- **kf3's host channels when the VMM is root** (the audit's P0, the same day). The owner's full
  sentence: *"I dont think you should refuse cap sys admin if clearing a bit fixes it. Sandboxing vmm
  is not our job though."* ⇒ kf3 does not refuse to run with `CAP_SYS_ADMIN`. It clears the bit from
  a thread's effective set where host RM reads it: for each channel-alloc call kf-host makes, and
  for the life of each thread that calls libcuda. `design/THE_CONSTRAINTS.md` §30 has the rule and
  its checks (`v3-sec-nonpriv`, pending the owner's review).

## Q. The address model: what each kind of address may reach (2026-10-03)

- **Owner, verbatim:** *"Gpga offsets can't, outside guest vram is invalid. Gpa offset can't either,
  only reference guest ram or its bar or an error. Same for bar offsets. Passthrough uses a sandboxed
  channel, that strictly only maps based on pte/pdb and nothing else. Translated uses a channel only
  gpga and/or gpa is mapped."* And, on host-process addresses as GPU addresses (HMM): *"only for the
  cuda channel (vmm ptx refresher), and this offset is never supplied or given by guest."*
- **Rules that follow:**
  - A guest-VRAM offset (GPGA) outside guest VRAM is invalid. A guest-physical address (GPA) resolves
    only to guest RAM or the device's own BAR, or it is an error. BAR offsets are validated the same way.
  - A passthrough twin's host VA space maps only rows derived from the guest's own page tables. It maps
    nothing else: no windows, no kayfabe memory. (Audit S1-21 records where the code departs from this today.)
  - A Translated channel's host VA space maps guest VRAM and/or guest RAM, and nothing else the guest's
    work can address. **This answers `OWNER_QUESTIONS_2026-10-03.md` 6c: a guest-RAM window in the
    Translated space is allowed.** The one mapping the engine needs beyond that, the Translated
    channel's own command ring, is kayfabe-authored, mapped read-only except its fence, and outside
    every address the rewriter will emit.
  - Host-process addresses reach a GPU only in kayfabe's own CUDA contexts (the walker and the display
    compose), and no guest value is ever used as one. Guest data only steers offsets inside those
    kernels, so the kernels' own bounds (the walker's store bound; the compose layer check) are
    host-memory-safety obligations. They stay in the audited set and need tests that can fail. HMM is
    not refused. (Audit S1-05 is narrowed accordingly.)
- **The rule is keyed on the channel's privilege, which kayfabe always knows.** Owner, verbatim: *"what
  you do know is if the channel is privileged or not. channel guest says is unprivileged cannot contain
  a full vram map ever, forbidden. real hardware must be told, so ogkm indicates it, as on bare metal
  channel userd and the ring and push buffers are mapped in an unprivileged cpu process, so the gpu is
  told any data at those addresses must be confined without ogkm checking it as it doesn't inspect in
  the first place. right? so then we should too. this is a critical flaw worth fixing right?"*
  - On bare metal, RM never inspects a user pushbuffer. Confinement is the hardware's job: the
    channel is unprivileged, and its VA space holds only what the kernel mapped for that process.
    kayfabe gives the same guarantee the same way. The host VA space that runs work from a channel
    the guest created unprivileged holds only rows derived from the guest's page tables: never a
    window, never kayfabe memory. The guest's RM sets each channel's privilege, and kayfabe services
    the channel allocation, so the decision needs no knowledge of what the space will later hold.
  - Audit S1-21 (the windows mapped in every mirrored space) breaks this rule and is **critical**. The
    fix is P1 + P2: no window in any space an unprivileged channel uses, and Translated work in its
    own space as above.
- **Also the same day:** *"We must check iova addresses are supported in kayfabe for guests requiring
  iommu protection. Not that this becomes a hard retrofit later."* A guest DMA address (an IOVA under a
  guest vIOMMU) is a fourth kind. It must be translated to a GPA at one validated boundary before any of
  the rules above apply. The readiness check is on `v3-viommu` (`design/V3_VIOMMU.md`).

## R. Memory safety: the `_unsafe` file is the audit perimeter (2026-10-03)

- **Owner, verbatim:**
  - *"safe code cannoy hold a raw pointer, either get rid of it or declare it as unsafe code and
    ensures it validates if safe code callers use it, do whats best"*;
  - *"we need to limit the amount of unsafe code, just logic may not need unsafe code"*;
  - *"the main invariant to hold, to protect against memory bugs, is that the vast majority of safe
    code that contains a bug and calls into any unsafe code is bound checked into that function. not at
    every call site. its a protection at us, a few validation sites is easy to audit, at every call its
    not then its basically unsafe code declared as safe."*;
  - *"if you would export an abritary read memory at vmm offset + length address as normal rust
    function, unchecked, and then expect safe code to call it correctly, you basically have a function
    that can dereference pointers without unsafe { ... }. so it breaks the compile time security rust
    provides. ideally by only auditing unsafe code we can conclude that with rust + that audit (assume
    the audit was perfect) that memory bugw cannot occur, without an audit on safe code. the same way a
    memory bug should not be possible in java/python/js/C# code ... i would say if the file ends with
    _unsafe, unvalidated length+offset or raw ptr is allowed, incl validation functions, even though you
    didn't need rust's unsafe block. then if safe code calls it, its validated, and that file gets a lot
    ot audit attention. so unsafe block is used as I needed this otherwise rust doesn't compile, if its
    not needed dont use it even in an unsafe file. unsafe file means this file can violate memory
    safety. so unsafe rust block not allowed in safe files is one gate, but not the whole actual
    security promise for memory bugs."*
- **Rules:**
  - A file named `*_unsafe.rs` is the audit perimeter: "this file can violate memory safety". It may
    hold raw pointers, unvalidated offsets and lengths, and the validation code itself, including code
    that needs no `unsafe` block. Code outside the perimeter must be memory-safe for every input by
    construction, so it needs a logic review only, never a memory-safety audit.
  - Every function a perimeter file exports to safe code validates all of its own inputs: overflow,
    range in the real allocation, alignment and lifetime. A buggy safe caller cannot cause a memory
    error. A precondition left to callers is forbidden; it is unsafe code declared safe.
  - `unsafe {}` is used only where Rust does not compile without it, inside the perimeter too.
  - No raw or disguised host or device address outside the perimeter. The console frame's address
    (S1-03) and kf-cuda's device-memory functions (S1-04) are being converted on `v3-sec-rawaddr`.
    The owner keeps the console display feature.
- **Proposed by Claude the same day, and ADOPTED by the owner the same day** (*"Do your
  suggestions/you think is best as well"*):
  - (a) Inside the perimeter, mark a function with an unchecked precondition `unsafe fn` even when
    its body needs no `unsafe` block, so every caller states why its arguments are valid.
  - (b) Code that produces an address hardware will dereference (GPU page-table entries, RM ioctl
    structs, kernel launch arguments, `mmap` targets) is perimeter material even though it compiles
    as safe Rust.
  - (c) Ratchet the perimeter's size (lines and exported items), not only `unsafe` blocks.
  - (d) `kf3.c` is entirely inside the perimeter and must compile in CI.
  - Also adopted, from the same proposal: the CI gates that enforce this model.
    1. No `unsafe` outside the perimeter, in a form that cannot be bypassed (audit S1-01).
    2. No raw or disguised address outside the perimeter (S1-02; built on `v3-sec-rawaddr`).
    3. A reviewed table of every function the perimeter exports to safe code, each with the checks it
       performs and the test that shows each check.
    4. Private fields on handle types that carry addresses.
    5. No `Copy` or `Clone` on handles that own memory.
- **Standing merge approval** (owner, the same day, verbatim): *"you have full approval to merge only
  it ci passed, code is good and it passed on real hardware with a real boot and apps. you are free to
  merge in a candidate merge branch and then test that, or check per branch, what you prefer, as long as
  the end state as a whole in main is tested"* and *"then if thats true, you dont have to ask to merge"*.
  The bar for master, for code and security-policy changes alike: CI green; the code reviewed; and the
  exact commit master will point to has passed, on a box with a real NVIDIA GPU, a real guest boot, the
  merge bar (tests, gates, bare metal, guest suite) and the apps. A candidate branch that merges several
  lanes is tested as a whole, at its own head.
- **Testing before master** (owner, the same day): *"before you merge to master, test it works"*, *"on
  real hardware"*. Code reaches master only after CI and a merge bar on a box with a real NVIDIA GPU,
  at the exact commit, plus that change's own hardware tests. Docs-only commits are tested by CI.

## S. Privileged host management: stub; unprivileged features: implement; GPU work: never forge (2026-10-07)

**STATUS: LIVE, 2026-10-07.** Binding owner ruling, relayed by the coordinator during the Windows
Code43 work (`traces/windows_code43_walls_20261007/README.md`, runs 25-27). It refines §H.

1. **Stub.** A privileged host-management action that is not compute and has no display or compute
   effect on the guest may be accepted or armed silently. It keeps a coherent "feature absent"
   state and never touches the host. Examples: RC-recovery policy (`SET/GET_RC_RECOVERY`), and
   arming notifiers for thermal or cooler diag zones, the power connector, platform power mode, aux
   power, GPU RC reset and ucode reset. Guest queries in the same area stay coherent with the stub:
   refused or reported absent, never filled with invented values.
2. **Implement for real.** Anything kayfabe can do with unprivileged host access is implemented,
   not stubbed: LUTs, timeslice, display colour, and the virtual monitor including hotplug and
   resize. An event about such a feature is armed only if kayfabe can post it from its own
   state. Otherwise the gap is reported.
3. **Never forge.** GPU work that reached the hardware is never forged (§A.3, unchanged). An
   event about real GPU work (copy-engine or graphics completions, runlist preemption, P-state or
   power events) is accepted only if it is derived from real host events.


**§S applied to specific notifiers (owner, 2026-10-07, after the VFIO event census).** These
are the four indices the real GSP does post in vfio-8/9/10:
- **PSTATE_CHANGE (33):** a no-op. The host does power management. Arm silently, never post.
- **HDCP_STATUS_CHANGE (34) and AUDIO_HDCP_REQUEST (45):** there is no HDCP, and none can be
  forwarded. Arm silently and never post. Every HDCP-related answer reports it as absent or
  unsupported, consistently.
- **RUNLIST_PREEMPT_COMPLETE (139):** the host driver does real preemption. Arm silently for
  now. If Windows issues the preempt control (in the VFIO boots: FIFO_DISABLE_CHANNELS with a
  `pRunlistPreemptEvent`, first at RPC index 3307), serve it only as a real unprivileged
  preempt of the VM's own channel group, and post the completion only from the host's real
  completion. Never invent one.

**§S applied to guest-kernel GR work (owner, 2026-10-07, relayed by the coordinator after runs
29-31; `traces/windows_code43_walls_20261007/README.md`, run31 and the GR-tier sections).**
Windows' first kernel-channel work is FERMI_TWOD_A and KEPLER_INLINE_TO_MEMORY_B methods on its
kernel GR channel, plus a software-subchannel bind on its kernel CE channel.
1. **Unprivileged host channel only.** Kernel-GR work may run for real only on an UNPRIVILEGED
   USER host channel with access to the VM's mirrored guest memory and VA space. No privileged host
   channel, no privileged RM verb, no host action taken from guest bytes. At birth the host channel
   is verified and asserted USER, and the birth is refused otherwise (tested).
2. **Re-author from an allowlist.** The rewriter re-authors host methods from a per-class allowlist
   (starting with exactly the observed 2D and I2M methods; one class/method set at a time). It
   never copies guest pushbuffer words. Every address operand is validated as a virtual address
   inside the VM's space. A method outside the allowlist, a privileged-state method, and any
   unbounded read or emit is refused by name and kills only that channel. Hostile guest, guest root
   included. Default-off flag until proven.
3. **Native first.** Each new class/method set is validated on the borrowed host with the native
   oracle (real GPU completion, resources freed, host display healthy) before any Windows run.
   Strictly serial, one VM or oracle at a time.
4. **Software subchannel bound to a non-class value** (5-7): accept the bind silently and refuse any
   later software method on that subchannel by name (logged). The hardware behaviour this matches
   is inferred, not tested.
5. **No forged completion.** Completions come only from real GPU work on the host channel. No CPU
   executor for GPU work.
6. **Standing rules.** Power, thermal, process, preempt and P-state stay host-owned stubs; no HDCP;
   faults depend on the UVM plan (stop and report if reached); FIFO_DISABLE_CHANNELS preempt, when
   reached, only as a real unprivileged preempt of the VM's own channel group; no new emulated
   kernel channel without telling the owner.
7. **Completion data and interrupts (added the same day).** GPU completion data must come from the
   real GPU writing GP_GET and semaphores into guest-visible memory (kayfabe is not in the
   completion path). Completion interrupts are relayed host non-stall event → guest vector (the
   interrupt plane maps GR0 and the other engines). For the GR tier, verify and report both, include
   them in the native validation and in the README of the first run that executes GR work, and fix
   the relay before relying on it if it is missing.

⊘ *Confirmed by the owner 2026-10-07 (relayed by the coordinator), above the text it settles:* the
`PERF_GET_POWERSTATE` `AC` stub is fine as it is. The entry below is no longer an assumption; its
"owner to confirm" and "not an owner decision" are answered by this line.

**§S applied to `PERF_GET_POWERSTATE` (2026-10-07) — ASSUMED from the power ruling, owner to
confirm.** `NV2080_CTRL_CMD_PERF_GET_POWERSTATE` (`0x2080205a`) answers `NV2080_CTRL_PERF_POWER_SOURCE_AC`
(`kf_rm::vfguest`). This is treated as a §S.1 stub under item 6's "power … stays host-owned": the
answer is the vGPU-guest body's constant (`subdeviceCtrlCmdPerfGetPowerstate_VF`,
`ogkm-580.65.06: src/nvidia/src/kernel/gpu/perf/kern_perf_ctrl.c:293-308`), never a host reading,
and nothing reaches the host. §S.1 also says queries in a stubbed area are "refused or reported
absent", which an `AC` answer is not; the classification is the coordinator's reading of the
ruling (compliance audit `2026-10-07-code43-gr-derive-compliance.md` S6), **not an owner
decision**. If the owner rules otherwise, the control is refused instead.

**§S exception: a diagnostic BAR0-read trap (owner APPROVED 2026-10-07, relayed by the coordinator
after the run35 stop; `traces/windows_code43_walls_20261007/README.md`, "Stop: two runs after A and
B").** An explicit, scoped exception to "only BAR0 writes trap" (`AGENTS.md`, Rules;
`design/THE_CONSTRAINTS.md`):
1. **Default off.** Behind a flag (`KF3_BAR0_TRACE=1`). With the flag off, production behaviour is
   unchanged: BAR0 reads never exit.
2. **Scoped.** Reads trap only from a guest-kernel channel's `GPFIFO_SCHEDULE` (the last one before
   the teardown is the one that matters) to the first following `Free`.
3. **Bounded.** At most 4096 accesses recorded per run.
4. **What it logs.** Every BAR0 read (with the value served) and write in that window, doorbell
   writes included, plus a one-time dump of 64 words of the new channel's USERD and its error
   notifier.
5. **Nothing else changes.** A trapped read is answered with the value the untrapped read would
   have returned; nothing is forwarded to the host.
Implemented as `crates/kf-qemu/src/bar0trace.rs` (ABI 21's `kf3_set_read_trap`). A window there
closes at the next RPC's queue-head write, so each window lies inside the approved interval.


> **Correction, 2026-10-31 (provenance fix; applies to the whole of "§S applied to `0x2081010d`"
> below).** Three statements below rest on a closed-binary reading and are **withdrawn**: that
> `paramSize 0` and the export flags come from "the control's export row in the pinned retail
> driver", that the flags say the control is non-privileged, and that
> its id is "generated" from that driver by `tools/windows-ctrl-export/derive.py` (the tool is
> deleted; `crates/kf-abi/src/hoststub.rs` is now a hand-maintained, source-tagged table, and the
> flags and row position are no longer carried; nothing branched on them, they were only logged).
> What stands, publicly: the interface `0x208101` is OGKM 580.65.06 (`g_finn_rm_api.h:425`); the
> message `0xd` and the declared params size `0` are measured (`run41-requests.log:95` and `run53-requests.log:526` in
> `traces/windows_code43_walls_20261007/`; 5 of 5 real-GPU captures show request and reply
> of 0 bytes, `docs/design/tier_checks/tier2_compute_host.md` item 8). The control's *per-control*
> privilege is therefore **not known from any public source**; what OGKM shows is only the class
> level (`NV2081_BINAPI` is allocated non-privileged under a Subdevice and forwards its controls to
> the physical RM, `resource_list.h:460-470`, `binary_api.c:60-110`, per `tier2_compute_host.tsv`
> row `ctl-2081010d`). The owner's decision (a)/(b)/(c) in the "For the owner" bullet below is still
> open, but its premise "the retail flags say it is non-privileged" no longer holds: option (b)
> should be judged on the class-level fact and a measurement of the host's answer (a Linux client
> issuing it on an `NV2081_BINAPI` object), not on the flags. Behaviour is unchanged by this fix.

**§S applied to `0x2081010d` (2026-10-07) — ASSUMED from the owner's 2026-10-07 statement that
privileged non-compute actions can be stubbed; owner to confirm.** Owner, 2026-10-07 (relayed by the
coordinator): *"Most actions that are not compute, privileged actions can be stubbed."* and *"All about
power control, thermal and process, preempt management is host"*.
- **What is stubbed.** Exactly one control: `0x2081010d`, answered `NV_OK` for a Windows 580.88 guest
  (`kf_rm::hoststub`, default on; the default-off `KF3_DIAG_ZERO_OK` diagnostic is removed). Its id is
  generated (`kf_abi::hoststub`, `tools/windows-ctrl-export/derive.py`): the interface from OGKM
  580.65.06 `FINN_NV2081_BINAPI_INTERFACE_ID` (`g_finn_rm_api.h:425`), the message number 0x0d from the
  bisect, `paramSize 0` and flags from the control's export row in the pinned retail driver.
- **Why only this one.** Runs 36-41 (2026-10-07, `traces/windows_code43_walls_20261007/README.md`)
  answered 13 refused power/thermal/perf/clock-area queries with zero-filled `NV_OK` and bisected
  them: `0x2081010d` alone is sufficient for StartDevice to pass VFIO 2861; the other twelve are not
  needed and stay refused. The four with public layouts (`0x2080205b`, `0x20802068`, `0x20802801`,
  `0x00800106`) are among the twelve, so no invented or zero-filled reply is shipped for them.
- **What the answer claims.** Nothing but the status: the control has zero params, so "zero-filled"
  invents no value. Nine of the thirteen had no public layout at all; for this one even the meaning is
  not public. A web search (2026-10-07) found no public layout. vfio-10's reply bytes exist and are
  not shipped as a captured table (derive-never-capture).
- ⚠ **For the owner.** The retail export row says the control is `NON_PRIVILEGED |
  ROUTE_TO_VGPU_HOST | GSP_PLUGIN_FOR_VGPU_GSP` (flags `0x10208`), and its area (`NV2081` binary API)
  is not shown to be power, thermal or P-state. So "privileged host management" is an assumption about
  its purpose, not a reading of it. Options: (a) keep the stub; (b) issue it on kayfabe's own host
  objects for real (it is non-privileged, but its effect on the host GPU is unknown); (c) refuse it,
  which returns Windows to Code43 at VFIO 2861.

**§S applied to notifier 196, `USE_GC6_REDUCED_THRESHOLD` (owner APPROVED 2026-10-21, relayed by the
coordinator).** Arming the Blackwell GC6 reduced-threshold notifier (196) is a §S.1 stub: it is accepted
silently, no event is posted and nothing reaches the host. Reason (owner): power is managed by the host
driver and mostly by GSP, the guest does not manage it, it is not used by guest userspace, and the
system must remain unprivileged. §S.3's "real host events" for power would apply only if an event were
needed for correctness, which is unlikely; monitoring is never a reason to allow a privileged verb.
This settles the open question on draft PR 56 (the predecessor's S.1 reading is now the ruling, no
longer an assumption); promoting the code to master still needs its hardware verification.

**RE scope clarification (owner 2026-10-21).** Inspecting NVIDIA code that is not under an open-source
licence, compiled or decompiled (disassembly, Ghidra, decompiler output, disassembly of a crash dump),
is reverse engineering and falls under the RE rules (the constant ruling, kayfabe-private only, nothing
derived, no public references), not only the use of RE tools. Traces (interfaces, ordinals, RPC and
event logs, observed behaviour) are not about the code and may be inspected freely. Earlier private RE
work may be built upon. Following assembly of open-source-licensed code is NOT RE: the open GPU kernel
modules (ogkm) are OSS-licensed, so a disassembly of a full ogkm build, whether built by us or
distributed by NVIDIA, contains nothing proprietary and may be inspected freely. That applies only to
full ogkm builds (Linux guests), not to the proprietary driver, whose binaries stay under the RE rules. Only code whose licence is open source is covered: a binary
payload that merely ships inside OSS-licensed files (the GSP firmware as a C array, for example) is
not, because the file is open source but the meaning of the contents is not covered by that licence,
so inspecting the contents of such a payload is RE. When in doubt, it is RE (owner's safe side).

## T. Directives recovered from earlier sessions (filtered 2026-10-08)

**STATUS: LIVE, 2026-10-08.** Recovered from earlier sessions (an audit of the owner's messages,
2026-10-07; branch `docs/owner-directives-20261007`, commits `d548a7a8` and `49b99d1e`), FILTERED
2026-10-08 under the owner's rule (§W: later rulings win; v1/v2 directives excluded). Unmeasured claims
stay as the original states them and are marked as recovered. Of 13 recovered entries, 5 are kept
(T.1, T.2, T.3, T.11, T.12); the numbers are the original ones, because `traces/windows_code43_walls_20261007/README.md`
cites T.1. Every original directive, with its KEEP or DROP and the reason, is in
`docs/design/OWNER_DIRECTIVES_RECOVERY_20261008.md`. Each quote is copied verbatim from the owner's
message, typos kept, and `…` joins fragments of one message. The audit's message dumps are not in the
repo, so the date and time are the citation. An entry's date is the day the owner said it: ask whether
its reason still holds before relying on it (see the top of this file).

1. **The trusted test host (RTX 4070) is trusted hardware.** *(Recovered; the owner's wording was relayed by the
   coordinator. 2026-10-28 hygiene edit: the host's network address in the quotations below is replaced by "the
   trusted test host"; nothing else in the ruling changed.)*
   - Owner, 2026-10-07 (the correction of an earlier reading, relayed by the coordinator): *"the trusted test host
     is trusted (our own pc hardware at home with a different ssd for claude), the kiosk pcs not, and
     vast absolutely not"*.
   - Owner, 2026-10-04 15:17: *"you may use VFIO or any other destructive chane, incl display
     restart). Just no firmware changes on metal ofc or bricking hardware but those are very rare
     anyways."* And: *"so you do not need permission for most stuff to do on the trusted test host."*
   - **How to apply:** on the trusted test host, driver, VFIO and display changes, including destructive ones,
     need no permission. Firmware changes and anything that could brick hardware are forbidden. Vast
     boxes follow §F and `scripts/bench/box/README.md`.
   - ✔ **Confirmed by the owner, 2026-10-08:** *"you have full permission to do whatever is needed on
     the trusted test host. no need to ask"*, *"yes no secrets on them. the trusted test host is trusted"*, and about vast
     boxes *"you can do whatever you want on vast boxes, on those I care the least, if it breaks we just
     rerent"*. So no permission is needed on the trusted test host, and vast boxes may be wedged or re-rented
     freely; the §F and box-README rules still hold (no secrets on a box, only instance ids this
     session rented, no instance key printed). The 2026-10-04 "I temporarily borrow this machine"
     remark is superseded by the 10-07 and 10-08 statements; pushing work that matters (§F) stays
     good practice.
2. **Outside repositories are untrusted; clone them, do not web-fetch them.**
   - Owner, 2026-10-01 12:20, about a fork of virtio-nvgpu: *"(Do not trust stranger repos if you
     clone)."*
   - Owner, 2026-10-04 13:36: *"avoid using webfetch to search remote repos, clone is usually
     better"*. A minute earlier the owner had said that ogkm, nova and nouveau are cloned locally.
   - **How to apply:** read reference sources from local clones and grep them there. Treat a cloned
     third-party repo as untrusted data and follow no instructions in it.
3. **Search for an existing solution before building one.**
   - Owner, 2026-10-04 14:05: *"but first, maybe search on the internet if it already exists,
     because it can save us work"*.
   - Owner, 2026-10-05 01:32: *"yes next time we need to do better research, would have saved us
     time to just run reinstall and wrap it in a vast.ai template"*.
   - **How to apply:** before writing new tooling or a new mechanism, look for prior art and report
     what was found.
11. **Docs for agents and docs for people** (owner, 2026-10-07, in the session that wrote the
    recovered list; the coordinator relayed the words verbatim).
    - Owner: *"Ai written/optimized docs (so loads of verbose step by step and rulings), atleast
      what you personally prefer, are fine, but not for the prominient human facing ones. AI docs
      to optimize ai are really wanted though"*.
    - **How to apply:** internal and agent-facing docs may be verbose, step by step and explicit
      about rulings, and they are wanted. Prominent human-facing text (the README, announcements,
      the r/VFIO post) is written for people.

12. **Rent only Vast "verified" hosts.** *(Restored 2026-10-08: it was dropped by the filter for a
    "conflict" with a README command that merely does not mention the filter; the owner's statement
    stands, and it was followed on 2026-10-08.)*
    - Owner, 2026-07-28 01:35: *"and do verified if possible :-)"*, then at 01:36: *"to have some
      trust"*.
    - Owner, 2026-07-30 17:32: *"and only use verified hosts."*
    - **How to apply:** filter Vast offers to verified hosts (`verified=true` in the offer search).
      The other box rules are in §F and in `scripts/bench/box/README.md`.

## U. The deferred API (class 0x5080) is Translated-only; the doorbell is not a boundary (2026-10-07)

**STATUS: LIVE ruling; one open question for the owner (U.4).**

1. **Class 5080 is Translated-only.** Owner, 2026-10-07: *"So then my ruling will be api defer for
   translated only."*
   - **How to apply:** a class-5080 alloc on a Passthrough channel is refused `NOT_SUPPORTED`; on a
     Translated channel it is admitted, and the `0x200` trigger (`DeferredApiV2`, data = the
     `hApiHandle` registered with `NV5080_CTRL_CMD_DEFERRED_API[_V2]`) is serviced by a
     host-authored equivalent, never forwarded as guest bytes. There is no channel-creation flag that
     announces the class (ogkm `alloc_channel.h` `NVOS04_FLAGS_*`; the class is a `KernelChannel`
     child, `RS_FLAGS_ALLOC_NON_PRIVILEGED`, `resource_list.h:1525`), and a Passthrough ring cannot be
     converted mid-life, so the alloc RPC is the control point.
2. **The doorbell is not a synchronization boundary.** Owner, 2026-10-07: *"No diff at doorbell,
   doorbell doesn't provide any safe boundary for you other than translate and ring on host, it only
   provides "everything before this rung is now scheduled to run after it", it doesn't even provide
   a boundary to you to run code before a channel advances, as this can be done without doorbell or
   worse a doorbell of another process just sending the same token (everyone can write), plus perf
   wise its bad, doorbells are the most optimized piece of code. Nothing can be injected there."*
   - **How to apply:** ordering against a channel's progress exists only in streams kayfabe writes.
     A deferred `DMA_INVALIDATE_TLB` is therefore a host-written gate: an acquire on a host-owned
     semaphore at the `0x200`, released by the VA-manager thread after its diff is committed, then the
     host invalidate. Nothing blocks on a vCPU and no completion is forged. (Whether the Translated
     plane can emit that acquire is unverified; it is the first step of the implementation.)
3. **An unprivileged-origin deferred-API channel**, if one is ever needed, must not accept physical
   operands and must not map VRAM beyond what is assigned to the channel, as for Passthrough. Owner,
   2026-10-07: *"Such channels should absolutely not accept phys operands or map the entire vram
   beyond whats assigned to the channel much like passthrough."* Today every host channel is born
   with `DENY_PHYSICAL_MODE_CE` (`kf-host/src/channel.rs:234`); the VAS restriction (the channel's
   own VAS, not the GPGA VAS) is not built, and no creation-time signal exists to choose it, so such
   a use is refused until it is needed.
4. **Open for the owner.** The 2026-10-07 measurement (`traces/deferred_api_falsify_20261007/`,
   driver 595.91.07, not the 580 interval) shows a host client registers the guest's chosen
   `hApiHandle` verbatim (only `0`, the client handle, the firmware-reserved range and its own live
   handles are refused) and that two clients have independent namespaces. That is the owner's own
   condition for "match the handle, no translated handling". Ruling U.1 predates the result and is
   unchanged. Matching would still not cover entries whose side effects kayfabe owns (the TLB
   invalidate), and nothing in the scanned userspace needs the class (below), so there is no demand
   for the Passthrough route. **Owner to say whether U.1 stays as written.**

**Evidence behind U (measured unless marked).**
- No Linux guest run allocated the class: 250 trace files carry `AllocClassNotPermitted` lines
  (class 5080 was not on the allowlist before 2026-10-07); class 20608 (`0x5080`) appears only in
  Windows runs 16 and 17.
- A byte scan of the NVIDIA userspace libraries (595.91.07 on the host; 610.43.02 CUDA compat) found none
  of the four control ids `0x50800101..104` in the CUDA, compute, video, GL/EGL/GLX, Vulkan SC,
  `nvidia-smi` or MPS components; positive controls (`GPU_GET_GID_INFO`, `GR_GET_INFO`,
  `BUS_GET_INFO_V2`) hit. 580.x Linux libraries were not scanned. (Diagnosis only; per-file results
  were removed from this ruling 2026-10-31 under `docs/design/V3_PROVENANCE.md` section 2.)
- The same kind of scan on the Windows 580.88 driver package (`DriverVer 32.0.15.8088`, in the baseline
  Windows image; read-only, static presence only) found none of the four ids in the Windows CUDA, OpenCL,
  NVML and NVCUVID user-mode libraries, so the Windows CUDA user-mode path does not register deferred
  entries. **But the Windows graphics user-mode components do contain them** (inferred from the scan, not
  measured on the wire), as does the kernel driver. The earlier statement that Windows uses the class
  only from its kernel driver is therefore wrong for D3D; how the user-mode ids reach the RM (through the
  kernel driver) and on which channel kind is not measured. Occurrences of `REMOVE_API` (`0x50800102`)
  alone in unrelated components are not taken as use of the class (inferred: an unrelated constant).
- Of the eight commands the trigger can run, `DMA_INVALIDATE_TLB`, `GR_CTXSW_ZCULL_BIND`,
  `GR_CTXSW_PM_BIND`, `GR_CTXSW_PREEMPTION_BIND` are `NON_PRIVILEGED`; `GPU_PROMOTE_CTX`,
  `GPU_INITIALIZE_CTX`, `FIFO_UPDATE_CHANNEL_INFO` are `PRIVILEGED`; `GPU_EVICT_CTX` is kernel-only
  (ogkm flag words). Registration does not check the inner command; the check runs at trigger with
  the registrant's privilege (`deferred_api.c`). Execution on GSP GPUs is in physical RM (inferred).
- A stray `0x200` with no 5080 object raises Xid 32 on the firing channel only and the channel is
  RC'd; no MMU fault reaches nvidia-uvm. Not measured: cross-object, cross-channel and cross-client
  isolation, and rate (the probe's CE channel could not bind a software object).

## V. GPU UUID per VM and GPU; a VMM-neutral input trait; Translated channels forward unknown entries (2026-10-08)

- **GPU UUID** (owner, 2026-10-08): the guest-visible UUID is a hash of the host GPU's UUID and the VM
  id, one value per VM and GPU: stable enough across restarts, never the host's own UUID. The user may
  supply an explicit UUID per GPU (`gpu-uuid=`). Purpose as before: orchestrators must not see two VMs
  with the same GPU id (`docs/design/V3_GPU_UUID.md`). Of the four sub-decisions A-D there, the owner
  answered "a hash of host GPU UUID and VM id, per VM x GPU": that keeps the implemented identity
  source (`vm-id=`, else QEMU `-uuid`), the `slot` (PCI `devfn`) byte, and `auto` needing the host
  UUID. ⊘ Two points are the implementer's defaults, not stated by the owner: with no VM identity a
  random UUID for that boot with a named warning (B), and a refusal to realize when the host UUID is
  unreadable (D). Launchers in `scripts/` must pass `-uuid` so a VM keeps its GPU UUID.
- **Display broker and input** (owner, 2026-10-08): the broker's keyboard, pointer and cursor logic
  hooks onto the VMM through a full VMM-neutral trait. Policy (bounds, grab, absolute/relative choice,
  button and wheel routing, re-sync) lives in Rust in `kf-broker`; only a thin shim names the VMM.
- **Translated channels, unknown entries** (owner, 2026-10-08, "ok go ahead"; the oracle's evidence is in
  `traces/phys_operand_oracle_20261008/`, branch `claude/phys-operand-oracle-20261008`; still a DRAFT ruling): a known push-buffer entry in a Translated channel is
  inspected and its physical operands translated and checked; an unknown entry is forwarded as
  virtual-address-only. Conditions: the host twin is unprivileged with an address space that holds only
  that VM's memory; hardware refuses physical operands on an unprivileged channel for that engine class
  (measured per class, by the oracle; a class it does not clear stays allowlist-only); entry framing and
  lengths are bounds-checked from the push-buffer header for every entry; guest-controlled integer
  arithmetic panics rather than wraps (`overflow-checks = true` in the release profile); every
  forwarded unknown method is logged and counted. Isolation inside one VM is the guest kernel's;
  VM-to-VM and VM-to-host isolation is the host channel's privilege and address space.
  - **Evidence, 2026-10-08 (RTX 4070, driver 595.91.07, oracle rev `8c084ab7`):** [measured] on a
    `USER`-privilege channel the copy engine (class `0xC7B5`) refuses a PHYSICAL operand, source and
    destination, local FB and coherent sysmem, with or without the extra deny setting: Xid 32, channel
    reset, nothing delivered, while the VIRTUAL control is delivered. [OGKM reading, not measured] the 3D
    (`0xC997`), compute (`0xC9C0`/`0xC6C0`), video (NVDEC/NVENC/OFA) and host/FIFO methods have no
    physical-aperture operand at all, so a forwarded unknown entry there cannot encode a physical
    address and containment is the VA space. Not done: a privileged-channel positive control (no safe
    way to create one on that host; "honoured on a privileged channel" stays inferred).

## W. Answers to the 2026-10-08 open decisions

All owner statements of 2026-10-08, in the order the open-decision list was given.

- **nvkvm-pv broker diffs: apply** (owner: "yes apply"): keep the sub-pixel remainder per axis in
  `relptr_motion` (a relative delta below one pixel must not truncate to nothing), and hide the host
  cursor in `wl_set_grab` wherever the pointer is when the grab starts (one cursor, not two).
- **Colour: "No color correction" (610 SAT_MODE 3) versus mode 2:** *test and decide: we should prevent
  incorrect results.* Until the hardware test says it is the same, mode 3 stays refused.
- **Arbitrary chroma correction: keep it out** while apps do not use it and the screen renders correctly
  (it would not apply to screen sharing either); revisit only if a real workload needs it.
- **Segmented capability bits (`LOGNR`) for Windows: the implementer decides** (decision: not declared
  until a Windows run shows it asks).
- **`EVICT_CTX` under ruling B (§U):** a control kayfabe authors on the VM's own host twin, safe by
  construction; the owner's reading is that Windows shares OGKM's behaviour and does not depend on the
  eviction result in guest memory. Accepted as the direct act; "Windows never reads the context buffer
  back" stays an inference until a D3D run shows otherwise.
- **RUSD (`0x20800afe`/`0x20800aff`) and `utilization.gpu`:** the owner asks for GPU utilization to be
  served read-only (`nvidia-smi` is unprivileged on bare metal); power is not important. Direction: the
  values the guest sees are this VM's own, never host-wide quantities (§S exposure rule); the design
  and its measurements are open work (`docs/STATUS_AND_HANDOFF.md`).
- ⊘ **REVERSED 2026-11-15 (owner, §AT: "set the measured Windows profile as the default"): the
  software-runlist host-owned scheduling (`KF3_SW_RUNLIST_HOST_OWNED`, now no flag) is ON for a guest that
  declares the measured Windows driver, and off for every other guest.** "Undecided" was never a decision to
  keep it off, and the owner did not keep it off: every passing Windows gate ran with it on.
- **Software-runlist flag (`KF3_SW_RUNLIST_HOST_OWNED`):** ~~stays off and undecided~~ *(superseded
  2026-11-15, see the bullet above and §AT).*
  - ⊘ *Later idea, not for now (owner, 2026-10-08):* "for B ... kayfabe can quota VMs
    scheduling/fairness, is related." If kayfabe owns the scheduling of the host twins (B), a per-VM
    share, timeslice or fairness quota has a natural place; a guest-built runlist, which B ignores,
    could then never raise a VM's share. Whether the host exposes the needed controls to an
    unprivileged client is not checked (inferred open question).
- **`raw_control_native`:** under review, see the handoff; production code must not call it.
- **§T (recovered directives):** later rulings always win over recovered ones; keep only what is useful
  and does not conflict; **avoid directions from the v1/v2 kayfabe architecture** altogether.
- **Disks:** the 13 GB `base.qcow2` original under the old work directory `paguro-work.old` is deleted ("fully
  regeneratable"; the archive copy remains); `/workspace/nvidia-gpu-passthrough` is backed up to
  `/mnt/windows-work/archive/`.
- **Models:** Sonnet 5.5 by default, Opus 5.5 as the strongest tier (`CLAUDE.md`, *Models by risk*).

## Y. A diagnostic BAR0 trace mode with the VFIO reference's own tracer (2026-10-09)

⊘ Numbered **§X** on `claude/kf3-read-trace-20261008` (and in its code comments and `V3_BAR0_TRACE_MODE.md`); renumbered
§Y at the merge into `claude/windows-reset-20261009` (2026-10-09), because §X is the non-stall ruling of 2026-10-08.

**STATUS: LIVE, 2026-10-09.** Owner rulings of 2026-10-09, relayed by the coordinator. A second,
broader exception to "only BAR0 writes trap" (`AGENTS.md`, Rules; `design/THE_CONSTRAINTS.md`) than
§S's scoped window; it does not replace §S.
1. **What is allowed.** For diagnostics, kf3's BAR0 **reads may exit** (trap) to the VMM and be
   logged; boot slowness does not matter.
2. **Default off.** In every default, production and performance configuration the BAR0 mapping, the
   exit behaviour and the code path stay exactly as before: "only BAR0 writes trap; nothing blocks on
   a vCPU" stays the default. The mode is an explicit, loudly reported diagnostic exception, like the
   existing probe flags.
3. **The same tracer as the VFIO reference** (owner emphasis, same day: "using THE SAME tracer is the
   most valuable thing"; "inject the same PCIe tracer used for VFIO into kayfabe's device, so both
   give the same responses in the same formats"). Not a parallel look-alike: the observer is made a
   device-agnostic helper that both the patched vfio-pci and kf3 call, with the same properties and
   output, so the `scripts/bench/windows` harnesses and analysers work unchanged for both.
4. **Hostile guest bytes.** The trace has a hard cap on log volume (bytes and records), a drop counter
   reported at the end, and never grows memory or disk without bound per guest action.

Implemented (branch `claude/kf3-read-trace-20261008`, `docs/design/V3_BAR0_TRACE_MODE.md`): the
environment switch `KF3_BAR0_READ_TRACE=1` (ranges, caps: `crates/kf-qemu/src/readtrace.rs`), QEMU's
own `vfio_region_read`/`vfio_region_write`/`vfio_msi_interrupt` trace events called by `kf3.c`, and the
shared GSP observer (`tools/vfio-gsp-observer`, `gsp_observer_*`) behind kf3's `x-gsp-observer`
property.
## X. Non-stall interrupts wake every VM that armed the event; an edge is never dropped (2026-10-08)

Two owner statements on the same day; the second supersedes the gate/bucket part of the first.

- **First (2026-10-08):** remove the doorbell requirement from the Passthrough non-stall relay
  (`kf_chan::ptnsi`, branch `claude/passthrough-interrupt-20261008`): doorbells are guest-controlled and a
  weak boundary (the guest can ring to open the gate; the doorbell hook is slated to be replaced), and the
  1 s afterglow could lose completions. The host notifiers are GPU-wide anyway: RM delivers an engine's
  non-stall edge to every client registered on it, whoever's work it was. ⊘ *Superseded the same day:*
  this statement also asked for a live-twin condition and a per-vector token bucket; see the next item.
- **Second, binding (2026-10-08): follow NVIDIA — an interrupt wakes everyone.** Losing an interrupt for
  relevant work is a correctness bug; a cross-tenant wake is only a minor denial of service. So:
  - forward every host `FIFO_EVENT_MTHD` edge and every engine-notifier edge to every VM whose guest has
    **armed** that event (its own non-stall subscription, a host-recorded fact), not "has a live twin",
    not a doorbell, not outstanding work;
  - **invariant: an edge may be delayed, never dropped.** No dropping token bucket. Default: no pacing.
    Any pacing knob is env-tunable, default off, and loss-free (a pending flag per VM and vector, and a
    guaranteed trailing raise by a timer even if no further edge arrives);
  - keep the hostile-index refusal and the armed-event check, and `KF3_PT_NSI_RELAY=0` as the falsifier
    mode; counters: raised, coalesced-and-later-raised, no-armed-event;
  - the accepted residuals are written down: a tenant can make other guests wake more often (minor DoS,
    accepted, it is RM's own semantics) and a guest learns, by timing, that some tenant used an engine
    class it armed. Where: `docs/design/the_three_channel_kinds.md` §1.2 and `docs/FAQ.md`.

## AA. An MMU invalidate completes over table errors, as hardware does (2026-10-09)

**STATUS: LIVE.** Owner, 2026-10-09: *"we need to do as real hardware, if there is an error in the
tables we just complete anyways, not withhold it. On real hardware, if it hits misaligned leaves, it
just faults; if the driver would fix it, it has to do a new invalidate through the 3 channels v3
designated anyway. That prevents a guest-wide crash."*

- An invalidate whose spaces failed **only by absence** (a refused map, a walk-refused leaf) is
  **cleared**; the refusal stays counted and named (`absent_cleared`, bounded log). A GPU access to an
  absent leaf faults on that space's twin.
- ⊘ **Supersedes** the 2026-09-25 line "its invalidate is not cleared over leaves nobody mapped"
  (`crates/kf-mem/src/vasmgr.rs`, module comment, corrected in place).
- ⊘ **Not covered, still held:** a refused UNMAP (a placement the guest dropped may still be live on
  the host) and a refused invalidate — those are kayfabe's own failures, not errors in the guest's
  tables. Confirm with the owner whether they should also complete.
- ★ **Correction to the evidence below (2026-10-09, run 225, real GPU): the refused map was a kayfabe
  defect, not an error in the guest's tables, and is now fixed in the host verbs.** The refusal
  `Other(19313)` is `VA_STRADDLES_RESERVATION`: Windows maps rows that CROSS the 1 MiB edge of the
  twin space's low reservation (`0xb0000+0x80000`, `0xff000+0x2000`), a FIXED
  `NV_ESC_RM_MAP_MEMORY_DMA` names ONE `hDma`, and kayfabe refused instead of mapping. The GPU then
  used the unmapped VA and faulted (host Xid 31, `FAULT_PTE` at `0xff000` / `0xb0000`); the Windows
  driver died. `HostRm::map_kind`, `unmap_row` and `unmap_range` now SPLIT such a row at the
  reservation edges (`VaSpace::dma_pieces`): one map per `hDma` of the same memory object, all or
  nothing. "Cleared over absence" is therefore not the answer for these rows; they are mapped.
  Unit-tested only; the hardware verdict is the next Windows run.
- Evidence: run 223 (`inval=3138 cleared=3137 unreconciled=1`, walk refused leaves
  `refuse_mask=0x400`, then a TDR 0x117 after the guest polled `MMU_INVALIDATE`'s trigger bit
  forever); runs 221/190/181 refused one map (`0xb0000+0x80000: Other(19313)`).
- Opt-out for A/B: `KF3_INVALIDATE_CLEAR_OVER_ABSENT=0`.
- ★ **PROPOSED AMENDMENT for the owner (2026-10-14, `claude/win11-opengl-20261014`, NOT yet ruled) — answer to
  the second half: the misaligned 512 MiB "leaves" are not valid leaves; they are stale reads of recycled
  tables.** `[measured run 2990, e821bf3a, KF3_WALK_REFUSAL_DUMP, traces/win11_opengl_20261014/walkdump_excerpt.txt]`
  space `0xece42000`: PD2[5] = `0x0dcbee22` (a vidmem PDE) pointed at `0xdcbee000`, whose entries are 4 KiB PTEs
  (kind 6, valid, consecutive pages `0xdab2000, 0xdab3000, ...` and sysmem pages) — a page table, read as PD1;
  the pages they name are all-zero data pages (not PD0 tables: the coordinator's H1 is falsified), and 200 ms later
  the guest had repointed PD2[5] to a new PD1 table `0xdcbe3000` whose slots are empty (`0x20`) (H2, genuine
  leaves, falsified). Every burst on 0c5fedff/e821bf3a followed a process exit or an adapter reset. ogkm's own
  rule for a leaf-capable PD1 (`gmmuFmtEntryIsPte`: PTE VALID bit 0; Ada uses `kgmmuFmtInitLevels_GA10X`, PD1
  holds 512 MiB PTEs, VER2) is the walker's, so the decode is right; the refusal-as-absence (above) is the
  correct and harmless outcome, and no leaf support is proposed. Open, small: the burst repeats one log line per
  invalidate (count it instead).


## AB. The T-space is hardwired; only guest leaves in a Passthrough space; whole guest RAM only for privileged channels (2026-10-10)

**STATUS: LIVE, 2026-10-10.** Binding owner rulings, relayed by the coordinator. They follow the
window-exposure review (`design/V3_WINDOW_EXPOSURE_REVIEW.md`) and complete §Q's address model.
Implemented on `claude/hardwire-tspace-20261010` (GPU-free tests only; the hardware gate is in
`design/V3_TSPACE_HARDWIRED.md` §5).

**2026-10-12 amendment:** §AI permits only GPU-read-only private command rings
and staging for declared Windows user-work under its restrictions. Linux
Passthrough's prohibition remains. This supersedes the absolute wording in
AB.2/AB.3 only for that narrow exception; no writable private fence is included.

1. **Delete `KF3_TSPACE`; hardwire the T-space behaviour.** Every Translated channel runs in the
   T-space, which is built at prewarm. With no T-space, a Translated birth is refused by name; there
   is never a fallback to a mirror window or a mirror ring.
2. **In a PASSTHROUGH space nothing kayfabe- or host-owned may be mapped.** Every VA mapping
   there comes from the guest's page-table leaves, and only that. The only future exception is the
   not-yet-implemented sysmem-USERD address-size fix, and it must stay absent until it is ruled on.
   This applies to every mirrored space (twins, prewarmed spares, recycled spares). The legacy P5
   windows and ring region are deleted. A carve-out leaf is refused in every GPU mirror.
3. **Only TRANSLATED channels may have host-owned mappings** (windows, rings), and they live in a
   T-space that no Passthrough channel can ever use.
4. **Only PRIVILEGED (guest-kernel) channels may have the ENTIRE guest RAM mapped.** An
   unprivileged Translated channel gets nothing wider than what its own validated operands need.
   - As built: the T-space's windows and rings need a `kf_qemu::tspace::Privileged` witness, made
     only from the alloc's facts (`kernel_client && !user_work`). An unprivileged Translated birth
     is refused by name. No route produces one today.
   - Not built: the per-operand space such a channel would need.
- **Also the same day (coordinator, owner instruction):** keep every measurement, diagnostic and
  control flag (`KF3_BAR0_TRACE`, `KF3_MAPLOG`, `KF3_RPC_TRACE`, the display traces, `KF3_TCENSUS`,
  `KF3_PT_STALL_SNAPSHOT`, the GSP observer, the `KF3_NEGCTL_*` controls, …). Remove or hardwire
  only behaviour flags. Renaming to `KF3_DIAG_*` is a separate step that has not been asked for.
- Supersedes: §Q's "(Audit S1-21 records where the code departs from this today)" for the window
  half. The `KF3_TSPACE` / `KF3_INCA_REFUSE` "default off until box step 1" rollout in
  `design/V3_P1P2_TSPACE.md` §8 (corrected there). The `KF3_TSPACE` row of
  `design/V3_FLAG_INVENTORY.md` (its follow-up note is `design/V3_TSPACE_HARDWIRED.md`).

## §AC. Models, 2026-10-10

**STATUS: LIVE, 2026-10-10.** Owner: for the 5.5 generation Sonnet 5.5 is the default for everything;
Opus 5.5 only when Sonnet fails (use less Opus, weekly usage). Haiku 5.5 is to be tried after the CLI
upgrade, on mechanical lanes only (evidence collection, formatting, running a script, grep sweeps).
Supersedes the "Opus runs hypothesis work" half of `CLAUDE.md` *Models by risk*; correctness still
beats cost: a lane that fails on Sonnet is rerun on Opus, never accepted.

## §AD. An invalidate is never left incomplete for long, 2026-10-10

**STATUS: LIVE, 2026-10-10 (owner, in the Windows TDR session).** No MMU invalidate may stay marked
incomplete for longer than about a dozen milliseconds. A guest thread polls the trigger bit while
holding driver locks (run 223 was exactly that hang), and a suspended scheduler or a recovery waits
behind it, so the latency of the clear is part of the guest's own latency, not a background detail.
Consequences recorded by the coordinator (not yet implemented, measured first):
- The trigger-to-clear latency of every invalidate is measured and reported (distribution, max, count
  above 12 ms) in every run's status line and in the Windows and Linux lanes.
- Work that would exceed the bound (large rows at 4 KiB grain, budgeted refreshes, a map queued behind a
  long steer) must not hold the invalidate: it either completes within the bound or the design must
  make the host mapping ahead of the invalidate (the guest writes its PTEs before it triggers, so the
  mirror has the time between the PTE writes and the trigger). Clearing over a mapping that is not
  yet in place is only allowed under §AA (absence), never silently.
- `V3_BATCHED_MAP.md` §8.8 worst cases (a refresh budget of 2^21 grain calls, about 45 s of VA thread
  by the document's own estimate) violate this rule and need a latency-bounded design.

## §AE. No heuristics; a fallback is a clean mode, 2026-10-10

**STATUS: LIVE, 2026-10-10 (owner, Windows TDR session).** Avoid heuristics: no cap, threshold or knob whose only
justification is that it makes a symptom go away, because it can break other things and stays forever. A fallback must be a
CLEAN MODE: a complete, named configuration with a defined contract and its own tests, like the virtual scanout on Linux (a
whole alternative display path, not a tweak to the real one). Where a feature cannot yet be done right, the stock path stays
correct without it and the feature is added later (post-release if apps run without it, see the priority rule: anything that
prevents apps from running first). Consequences: the "fewer usable windows per head" idea for overlays is a diagnostic only;
the product answer is to fix the overlay flip completion, or to define a real single-plane display profile as a clean mode
with a derived, named capability, never an ad hoc cap. Priority rule recorded with it (owner, same day, his words): "I would focus on
anything that prevents apps from running. Anything that also works without, just like the virtual scanout on Linux and
decent perf, I tend to do post release." So: anything that prevents apps from running comes first; features that apps
work fine without (the owner's example is the simple virtual scanout Linux guests already use instead of the full display
engine) and performance beyond "decent" are post-release. Console fidelity (YUV overlay rendering on the console, z-order,
HDR on the console) falls under "works without".

## §AF. Per-die constants: guaranteed coverage, a published table, reuse of existing reverse engineering, 2026-10-11

**STATUS: LIVE, 2026-10-11 (owner statements in the Windows/OpenGL session; recorded by the coordinator, wording to be confirmed by the owner).**
- **Goal:** kayfabe must not support only the dies we could trace. The owner wants guaranteed, whole constant tables across a wide product
  range (the reference is `nvkvm-pv`'s breadth), because a project that only works on a few dies will not be adopted. A per-die trace taken
  by us, die after die, is not an acceptable coverage plan (one trace per die is a burden even for a few constants).
- **Order of sources for any per-die constant or layout:** (1) computed from kayfabe's own state or the class tables; (2) NVIDIA's open
  headers/docs (ogkm, open GPU docs) and every other open-source project that already carries it, **combinable**, pinned and licence-checked
  (nouveau, nova, envytools, gVisor nvproxy, ...): reusing existing community reverse engineering is preferred over doing our own, because
  those projects already span many dies and have users; (3) the host's own RM queried at runtime; (4) only for constants that remain, and
  only if the owner decides so after a legal/provenance check: reverse engineering of the GSP firmware or other blobs **to extract constants,
  not interfaces**, clean-room (a spec of facts is written; the implementation uses the spec), with the result **published as a table and
  treated as the truth**. The build and any derivation script must NEVER contain or require a decompile step: people compiling fetch the open
  repos and the published table.
- **Refinement (owner, later the same day): a combination of sources, with data that never comes from a blob.** The table is built from
  (a) NVIDIA's open repos and docs (ogkm, open GPU docs), (b) open-source projects that already carry the legal/provenance burden (nouveau,
  nova, envytools, gVisor nvproxy, ...), and (c) a **fleet of measurements on real GPUs rented on vast** (many GPU types), published as the table
  and treated as the truth. **Reverse engineering of the GSP firmware or blobs is used only to find issues and to confirm that the table and the
  behaviour are right** (verification), NOT as a data source, so no interface, layout or constant is derived from a blob. Every table entry
  carries a source tag (`ogkm`, `nvidia-doc`, `nouveau`, `nova`, `measured:<die>,<driver>,<date>`, ...); an entry tagged `blob` is refused by CI.
  This supersedes the "last resort: reverse engineering for constants" item above. Measured entries exclude anything board- or user-specific
  (UUIDs, serials, VBIOS/PROM contents, MACs).
- **Display is required** (owner): people want display, so a render-only/headless Windows mode is at most a fallback or experiment, not the answer.
- Our own traces remain valuable to find what must be answered and as test oracles for the die we own; they are not the coverage source.
- Supersedes for this question the diagnosis-only wording in §S/`feedback_reverse_engineering_boundary` only to the extent stated above:
  reverse engineering to extract constants is allowed in the last resort; interface extraction and decompilation inside the repo are not.

## §AG. Reverse engineering stays small and private; the provenance policy is public, 2026-10-11

**STATUS: LIVE, 2026-10-11 (owner statements; recorded by the coordinator, wording to be confirmed by the owner).**
- Reverse engineering (decompiling, tracing a closed binary) is allowed for **interoperability, diagnosis and verification**, kept **small**,
  and its output **is never made public** (private repository `reindertpelsma/kayfabe-private`). Reason (owner): this prevents the largest risk,
  spreading annotated IP. It helped the project and is not use of leaked sources (owner).
- The provenance **policy** itself is public, for transparency: `docs/design/V3_PROVENANCE.md`. It states rules, sources and checks, never what was
  analysed or found, and it must not claim a clean-room process (the same people and agents did diagnosis and implementation).
- Consequence for data: constants and interfaces come only from the sources in §AF; a diagnosis finding is turned into a behaviour-level
  verification item and confirmed from those sources or by measurement before code depends on it.
- To do: one lawyer review of the provenance page and of the overall reverse-engineering position before the public release; a sweep of the full
  git history for analysis material and closed-binary offsets (release gate).

## §AH. Per-die constants: the tier order and the per-die default, 2026-10-11

**STATUS: LIVE, 2026-10-11 (owner statements; recorded by the coordinator, wording to be confirmed by the owner).**
- **Tier order** for any value that is not derived: (1) derive/compute (ogkm incl. generated and Windows-specific code, from ANY ogkm version, nouveau/nova, other open
  projects, kayfabe state); (2) unprivileged host ioctl at startup; (3) **privileged measurement in a sweep** (root script, kernel module, hardware
  access) before any reverse engineering; (4) reverse engineering, only for the residue. An item moves down a tier only with a written reason.
- **Unknown things are not assumed to be per family** (owner): the default is measured per die and per firmware/driver branch; a family rule needs
  every measured die to agree plus a mechanism; an unmeasured die or branch gets the clean fallback, never a neighbour's row.
- The strategy, plan and unverified assumptions: `docs/design/V3_CONSTANT_STRATEGY.md`.
- Agreed additions (owner, 2026-10-11): demand filter before the tiers, delegation to the host driver where safe, a supported driver set, and an opt-in
  report of refused controls (`V3_CONSTANT_STRATEGY.md` §7).

## §AI. Windows user-work ring exception; Linux Passthrough unchanged, 2026-10-12

**Owner diagnostic authorization, 2026-10-12:** after the Windows experiment,
"you may also do a linux run with a flag to force the 'migratable passthrough'
even if linux doesn't need it" because ogkm is easier to debug. This permits an
explicit opt-in Linux lab experiment, not a change to normal Linux Passthrough.
Keep the native USER/own-VAS permissions and physical-operand prohibition.
Current diagnostic source accessor is Sysmem-only: either construct a suitable
Linux test allocation or explicitly implement/test vidmem access before claiming
ordinary Linux desktop channels exercise it. No such forced run has occurred yet.


**Subsequent owner clarification, 2026-10-12 (USER authority):** native USER
privileges and the channel's own explicitly admitted, refreshed GPU mappings
are the access boundary. Do not map all guest VRAM into an unprivileged channel.
Host-owned ring/staging addresses grant no extra execution authority; correct
userspace follows its mapping and synchronization contract. A malicious mutation
of its own ordinary commands is not, by itself, a reason to snapshot every
pushbuffer. Evaluate scan-from-first-submission with ordinary GP references
unchanged and staging only for intercepted triggers. This supersedes the blanket
immutable-copy requirement below; it does not waive independent validation of
host software actions or establish that native RC/reset stays contained. Actual
ordering, lifetime and containment still require tests. No PB parsing on a vCPU;
Linux Passthrough still forbids host-owned GPU mappings.

**Latest owner clarification, 2026-10-12 (three-mode dispatch):** the owner
requests direct native-token forwarding; bounded pre-mapped USERD/GP-descriptor
copy with a worker fallback for large deltas; and always-worker dispatch for
Translated, Emulated or migration-capable channels. No pushbuffer inspection on
a vCPU, no mapping/native ioctl there, and only a conditional nonblocking worker
wake syscall. MMIO and ioeventfd must use the same dispatch semantics. Review
atomic interleavings rather than assuming lock-free means race-free. This
supersedes the worker-only/inspection-from-birth requirement below to that extent;
the Linux host-mapping prohibition and Windows read-only mapping scope remain.

**Implementation interpretation, not a new owner ruling:** copying ordinary
user GP pointers does not require CPU interpretation of their pushbuffers while
the native channel remains USER, physical operands remain forbidden and its own
restricted GPU address space protects host resources. Once software migration is
possible, worker inspection must cover its parser history before accepting it;
the later owner clarification above permits trigger-only staging. The atomic
switch must exclude old CPU publishers; if any opaque native stream was already
published, native parser/binding/ordering continuity is a separate proof gate.
Holding a registration reply or observing empty GET/PUT does not close that gate.
A never-published channel may switch after all prior CPU claimants have exited.
See [architecture review](design/V3_WINDOWS_USER_RING_RELAY_ARCHITECTURE_REVIEW_20261012.md).

**Owner clarification, 2026-10-12:** "no host owned maps should remain for linux
 guests, but can be lifted for windows if its the only thing fixing the migration
 linux doesn't use". The owner also authorizes necessary Windows-specific rule
 changes, with an afterwards report of what changed and why. This preserves the
 Linux prohibition; it does not require a broader mapping exception. The current
 implementation target remains the read-only command mappings below, because
 the native reference mechanism works without a GPU-writable private fence.

USERD sysmem placement is a useful routing fact, not the sole admission rule:
allocation requests and declared OS identity are guest-controlled. Keep explicit
Windows user-work scope plus ownership/permissions checks; those checks must
remain safe even if a different OS declares that identity. An observed Linux
layout is not proof that Linux cannot request another layout.

**STATUS: LIVE, 2026-10-12 (owner approval in the Codex session).** Asked about a
revision of V3_WINDOWS_USER_RING_RELAY.md permitting only GPU-read-only private
command rings/staging for declared Windows user-work in its own restricted space,
worker-only submission and command inspection from birth, the owner answered:
"yes approve, as long as linux passthrough forbids host owned mappings".

- The exception to §AB.2/AB.3 is limited to those GPU-read-only command mappings.
  Linux Passthrough continues to forbid host-owned mappings. Kernel Translated
  spaces and the physical-operand prohibition remain unchanged.
- Reserved addresses prevent collisions; GPU PTE permissions must enforce write
  protection. The guest may read these mappings: do not put host secrets there.
  No writable private fence/gate, whole-GPGA mapping or privileged host channel
  is authorized by this exception.
- Submission remains on workers; no inline copy or new BAR1/BAR2 trap. Inspect
  immutable command copies from birth; a held registration reply does not drain
  previously submitted work or prevent hostile prepublication.
- This authorizes development of the revised design, not a compatibility claim.
  Native cursor, lifetime and ordering semantics still require evidence, bounded
  models, independent review and exact-revision Linux/Windows hardware tests.

## §AJ. The broker owns the whole cursor plane, including true XOR inversion; kayfabe composes no cursor, 2026-10-17

**STATUS: LIVE, 2026-10-17 (owner, in the coordinator session; recorded by the cursor-xor agent).**

- **D2.** An XOR cursor (the Windows I-beam, an inverting cursor) is composed BY THE BROKER as a true inversion of the pixels under it. The approximation (black fill, white
  outline, `nb_cursor_xor_approx`) is a stopgap and the fallback when composing is impossible (e.g. the frame is not readable), logged once. Kayfabe keeps sending the
  inversion mask with `CURSOR_F_XOR`; composition does not move back into kayfabe.
- **Kayfabe composes no cursor at all.** The pre-stream composing path (the XOR compose kernel, hiding the host pointer while an XOR cursor was composed, the COMPOSED/REFUSED
  cursor logic in kayfabe's display code) is the wrong direction and is to be DELETED, not kept as the fallback. This **supersedes** the sentence in
  `V3_BROKER_ROADMAP_20261015.md` section A ("the old composing path stays as the fallback ... until the owner decides to delete it": the owner has decided) and the
  "stays the whole story for a broker that does not advertise the stream" sentence of `V3_DISPLAY_CURSOR_FORMATS.md` §8; both are corrected in place.
- A broker without `CAP_CURSOR_STREAM` running on a test VM (it was kept only because it worked) is not the design. After the deletion such a broker shows the guest's own cursor
  or none; kayfabe never composes.
- The deletion is a separate change with its own tests and a Linux broker retest (list: `V3_BROKER_XOR_CURSOR_20261017.md` §6). This task did not delete code.
- D3 (same day): the cursor is oversized on the owner's screen; the size rule and fix: `V3_BROKER_XOR_CURSOR_20261017.md` §1.
- **D2 refinement (2026-10-17, relayed by the coordinator; binding).** The owner prefers to avoid the wl_shm tier and any unnecessary CPU copy of large frames in the present path, and to use GPU APIs when composing if that is possible.
  Built accordingly: the broker's XOR composing does not depend on the shm tier (it reads only the cursor's rows of a sealed memfd or of a LINEAR system-memory dma-buf; tiled or device-memory frames fall back to the approximation,
  logged once per change); a GPU path was evaluated and not built (`V3_BROKER_XOR_CURSOR_20261017.md` §8: it would put EGL/Vulkan and a vendor driver into the privileged broker; owner decision if it is ever wanted).
  The wl_shm presentation after a broker restart was a false trip of the VMM's acknowledgement detector, not a display refusal: `V3_BROKER_PRESENT_TIER_20261017.md`.

## §AK. Guest flips are never parked; the display rate is bounded by copies; the tick source falls back to the free-running clock, 2026-10-17

**STATUS: LIVE, 2026-10-17 (owner, in the coordinator session; recorded by the display-pacing agent).** Design and phase-1 code: `docs/design/V3_DISPLAY_PACING_20261017.md`. These amend §M D1 and bind
the display plane.

- **(a) Flips are never parked to bound the rate.** The guest's immediate (async / tearing) flips latch and complete at once; vsync flips complete at the next vblank. The display and
  copy rate is bounded separately: kayfabe copies only the NEWEST latched scanout frame, at most once per tick (mailbox semantics), into a free ring slot. §M D1's gate (a second tearing flip waits
  for the next tick) is **superseded** by this and removed.
- **(b) Flip syncing is lock step between guest and kayfabe.** The broker will later send a frame request only after it received the previous frame (credit based). Not built in phase 1; the
  design must not preclude it (it does not: the mailbox's tick credit is the place a broker credit plugs in).
- **(c) The free-running clock is kept.** It is the tick source when no broker is connected (headless) and the FALLBACK when the broker is silent. If no broker tick event arrives for 50 ms
  the free-running clock takes over so the guest's vblank and flips keep going; the moment a broker tick event arrives the broker is the tick source again, automatically. The guest-visible vblank
  is **never hard-gated by the broker**: a hung, hidden, occluded or crashed broker must not freeze the guest (Windows resets the display driver after about 2 s without a flip completion).
  Handoffs are seamless: one tick source at a time, the vblank counter and timestamps monotonic, no double tick closer than a fraction of a period (built as one half), the clock phase
  re-aligned to the broker's.
- **(d) Later phases (design only, not built):** the broker as tick source over the broker protocol (a capability bit; presentation feedback; commit every refresh so frame callbacks
  keep coming); an HDMI-over-TMDS virtual connector to lift the 75 Hz EDID cap and the 165 MHz DVI pixel clock (§M D3).

## §AL. Kayfabe enforces the lock step and never trusts the broker's timing; the flip system is testable locally, 2026-10-17

**STATUS: LIVE, 2026-10-17 (owner, via the coordinator; recorded by the phase-2 agent).** Design and code: `docs/design/V3_DISPLAY_PACING_20261017.md` §11. Binding on the broker-as-tick-source work (§AK (b)–(d)).

- **Kayfabe enforces.** A broker tick / `WANT_FRAME` that arrives at the wrong moment (early: the previous copy or send still in flight, inside the half-period guard, out of sequence) is not acted on then; kayfabe acts
  when the lock-step state allows (copy finished, slot free, sequence matches) — after the right moment, never before. Too-fast broker events never make kayfabe go faster and never build a backlog: credits coalesce
  (at most one pending, extras counted and dropped), the vblank is capped by the double-tick guard, frames in flight are bounded by a named constant with a test. A flooding or hostile broker costs counted, dropped ticks.
- **The guest's immediate flips are still never parked and never wait for a credit.** The rule governs the kayfabe→broker frame stream and the vblank clock only; the 50 ms silence fallback still applies.
- **Local tests of the whole flip path**, GPU-free: an exhaustive bounded model, a seeded soak (>= 100 000 flips per scenario over >= 50 seeds, an ignored 10 M-flip run), a real-socket run with a fake broker, floods of every
  kind, and broker STALLS (silence, partial messages, a slot never released, flapping around 50 ms, SIGSTOP); mutants of the harness itself. The harness's limits are stated in the doc (§11.7).
- (Added by the agent, flagged for the owner: after a fallback to the clock the broker takes the tick back only after proving itself — one good gap, then 2, 4, 8 — so a broker flapping around 50 ms cannot flap the source
  per tick; decision (c)'s "the first broker event takes it back at once" holds for a broker that never fell back. §11.5.)

## §AM. Guest vidmem is scrubbed by trust boundary: only a guest reboot scrubs, the whole store, GPU-side, 2026-10-18

**STATUS: LIVE, 2026-10-18 (owner, via the coordinator; recorded by the boot-layer agent).** Binding. Code and
tests: `crates/kf-qemu/src/scrub.rs` (the event table), `memreset.rs` (the memory stage's driver), `reset.rs`
(the decision), branch `claude/bootlayer-20261018`. The boot-layer rule (what the monitor shows) is separate:
§AM.5 and `docs/design/V3_DISPLAY.md` §4.11.13.

1. **Premise (owner-accepted, NOT verified by us):** the host driver does not promise zeroed memory, it promises
   *process isolation* — an allocation can only hold stale bytes of the SAME process, never another process's
   (like anonymous `mmap`; an RM `hMemory` behaves the same). A fresh QEMU/kayfabe process therefore gets a fresh
   store, and a crashed previous QEMU's store was cleaned by the host on process death.
2. **The only place a stale-bytes leak exists** is where the SAME process reuses the SAME store for a NEW OS
   instance: a guest **reboot** (a system reset delivered on QEMU's main loop: reboot, reset button, bugcheck or
   triple-fault reset, QMP `system_reset`). That scrubs: **the whole store `[0, fb_length)`**, on the GPU
   (`cuMemsetD8` on the store's device mapping, chunked, the completion awaited), before the next life runs and
   before anything from the store is shown. The firmware carve-out is NOT excluded: the guest's CPU views and GPU
   leaves reach it (counted, not refused), so only our two root pages — rewritten by the reset's own stage after
   the scrub — are kayfabe's.
3. **No scrub** at: VM start (isolation premise; realize still zeroes what kayfabe itself needs zero — the roots
   and the boot framebuffer `[0, G)` — and awaits that zero), VM teardown, an in-guest driver unload, a TDR-style
   reset, a bus reset the guest itself issued (a vCPU thread), an S3 wake, the basic-display → NVIDIA handoff. The
   same OS and root can already read all of its own vidmem; a real GPU does not scrub there either.
4. **Distinguishable:** the device tells a reboot from an in-guest reset by where QEMU asked from
   (`current_cpu != NULL` ⇒ a vCPU thread ⇒ the guest's own act) and the reset type (`RESET_TYPE_WAKEUP`). Were
   they not distinguishable the rule would be the lazy one: scrub on every reset.
5. **Fail closed:** a scrub the host refuses, or whose completion it refuses, is never acknowledged: the reset
   gate stays closed, the stage retries the scrub alone with a back-off, and the device does not open on an
   unclean store (`docs/design/V3_DEVICE_RESET.md` §6 item 14). A reboot that arrives while a guest-issued reset
   runs still scrubs (a dirty bit, not a one-shot mark, drives it). **The store counts as dirty as soon as the gate first
   opens** (the guest runs from there, and its BAR1/BAR2/PRAMIN/DMA writes are invisible to the device; owner, 2026-10-18),
   so every system reset after the machine-creation reset scrubs, even if no BAR0 access was ever admitted; only that
   creation reset (pristine) is skipped.
6. **Boot layer (display policy, not a reason to scrub):** the boot layer is shown only while store `[0, G)` is
   clean; a guest RM life (fn 72) taints it, only a completed scrub of the range cleans it. See §4.11.13.
7. **Not yet measured:** the scrub's wall time (the log line carries it); hardware verification is the checklist
   in the branch report.

## §AN. The Vast CI Worker is test infrastructure, not a product; right-sized, not hardened further, 2026-10-25

Owner, 2026-10-25 ("we dont make SaaS"):

1. The Vast CI Worker is **TEST infrastructure** for GPU validations on every commit and release. It is not a
   product. It stays, is right-sized, and is not hardened beyond that.
2. The **Cloudflare Access pair is kept** (a free anti-bot gate, not the security boundary). The **reaper stays on
   Cloudflare**.
3. **Private manual GPU sweeps** use `scripts/ci/vast_simple` (draft PR #137: SSH pull, no Worker).
4. **Passing tests are not security.** The gate is the owner's approval of the SHA. The real risks are Vast key
   custody and spend/abuse. See `ci/vast-broker/docs/THREAT_MODEL.md` ("Why a Worker (and when not)").
5. **Budget:** the daily Vast budget is raised to **12 USD for 2026-10-25**; the **8 USD credit reserve is kept**.
   Per-lane stops live in the lane files.

## §AO. Goals recorded for the public goals page: hypervisor independence, arm64, multi-GPU, 2026-10-27

Recorded 2026-10-27 from the owner's instructions of 2026-10-22 (roadmap order) and 2026-10-27 (the goals page), so the
public page `docs/howto/goals.md` has a repository source for each item. These are goals, not commitments.

Each item below is labelled: **owner priority** (what the owner said), **design fact** (read from the source) or
**measured status** (what a record shows). ⊘ *Correction 2026-10-28:* an earlier version of item 2 ended with "Neither is
started before the family matrix is green". The recorded instruction is a priority order, not a start embargo, and the
sentence was removed. Thin multi-GPU tests and aarch64 builds already exist (item 3).

1. **Hypervisor independence is a goal, not implemented.** *(Owner priority, 2026-10-27: the goal. Design fact: the
   core is VMM-agnostic by design, see `design/THE_ARCHITECTURE_v3.md` version-axis table, row V, and
   `design/THE_DESIGN.md`; the `kf-core` crate has no QEMU dependency and reaches the hypervisor through its `Vmm`
   and `HostOps` traits. Measured status: only QEMU is supported today, and a second hypervisor needs a shim that
   does not exist.)*
2. **Priority order after the families.** *(Owner priority, 2026-10-22.)* All GPU families first, Blackwell included.
   Then multi-GPU guests (several kf3 devices in one guest, on Windows too, and choosing which GPU serves the
   display) and arm64 (see §K for the aarch64 rulings already made) as the next priorities.
3. **Implementation status of those two.** *(Measured status.)* What is measured for multi-GPU stays as
   `STATUS_DETAIL.md` section 3 records it: a Linux thin guest with two distinct GPUs. CUDA in a two-GPU guest, UVM
   across GPUs, peer access (refused by name) and Windows or display with several GPUs are not measured. Existing
   two-GPU thin tests and the aarch64 cross-builds do not establish full support.

## §AT. Proprietary-image runner: a list of images, prebuilt now and built later, state.json optional, clock and network asserted, 2026-11-16

Owner, 2026-11-16: the runner takes any number of images (a list); the kinds are `prebuilt` (phase 1) and `built` (phase 2, after the release run); `state.json` is optional (required for CI); a faked clock requires the network off and a networked guest requires the real clock (asserted in every launch path); the Worker builds instructions from its image and app configuration. Design, schema and the phase split: `docs/design/V3_PROPIMAGE_RUNNER.md`.
