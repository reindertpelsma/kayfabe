**STATUS: RESEARCH, 2026-11-08.**

# Code size and the boundary to a second VMM

**MEASURED:** AU1 audited the fixed source revision
`e9408ce4bf3f7956c74309d7b49e4b3e3230479d` on 2026-11-08. That revision was
`origin/next` at the initial fetch; it is an ancestor of the later `origin/next`
`8d495e905331e1a6fbd0acdb0c483e0c25786032`, merged into this audit branch in
the fact-check follow-up. The measurements remain pinned to `e9408ce4`, not to
the later branch head. This change adds this document and two JSON inventories.
No GPU, KVM guest, rental, deployment or external private
source was used. Public VMM documentation was read, not its implementation run.

**CODE READ:** The relevant owner rules are [the constraints](THE_CONSTRAINTS.md)
and [owner rulings](../OWNER_RULINGS.md), especially A, Q, R, V, AB, AI and AM.
The [architecture map](../../ARCHITECTURE.md) is a useful crate inventory, but
the source decides what is actually compiled or called.

**INFERRED — answer:** The tree implements substantially more than a PCI shim:
GSP/RM emulation, driver-version ABI selection, three channel paths, GPU page-table
reconciliation, virtual NVDisplay, boot display, and a broker. There is small,
demonstrable residue and a misleading source-count boundary; this audit does
**not** establish that a large fraction of the product can be deleted. A second
Linux/KVM VMM is feasible without replacing those GPU planes. It needs more than
implementing the existing `VmmOps` trait, and an out-of-process frontend has an
additional, unresolved mapping problem.

**MEASURED — scale:** The requested literal counting rule yields 186,031 code
lines in 23 workspace crates, of which 36,812 are in the specified generated
directory. A second, compiler-selected library-source estimate excluding whole
`cfg(test)` items is **160,562 code lines**, including the same generated
directory. It puts the QEMU Rust runtime at **29,092**, rather than the literal
rule's 22,809. Neither number measures linked binary size.

## 1. Hypotheses and what would disprove them

| Label | Hypothesis, with falsifier stated before the check | Result |
|---|---|---|
| INFERRED | Generated data and feature scope explain much of the apparent size. Falsifier: most retained source is unused or repeated application logic. | The largest identifiable buckets are ABI data, emulation, display and validation; no broad dead-code fraction was established. |
| INFERRED | The four crates examined for portability are independent of QEMU (the claim sources differ; see section 7). Falsifier: a QEMU dependency or executable QEMU API reference in their closure. | Dependency check supports QEMU independence; it does not establish a complete frontend abstraction. |
| INFERRED | `userpump` duplicates `user_turn`. Falsifier: production delegates its per-entry state machine to `user_turn`. | Falsified: `PumpIo` implements `user_turn::Io`, and the pump drives the shared `Relay`. |
| INFERRED | The unused CUDA upload helper and old self-test entrypoints can leave the v3 API. Falsifier: a workspace/feature caller or a compile failure after scratch removal. | Caller searches and scratch checks are recorded in section 5. |
| INFERRED | vfio-user is merely a transport substitution. Falsifier: retargetable CPU windows, read-only shadow semantics or asynchronous exits cannot be represented by the client. | Not established; the window backing is a material extra contract, section 10. |

## 2. Counting rule and per-crate inventory

**MEASURED:** `AU1_count.py`, its tests, JSON inventory and logs are in the AU1
report artifacts, outside the repository as requested. The per-file results are
also published as [literal counts](evidence/v3_code_size_20261108/literal_counts.json)
and [library-source counts](evidence/v3_code_size_20261108/library_counts.json);
both identify the fixed source revision and carry source-blob SHA-256 values.
The library inventory also includes the excluded test-item line spans.
The script can read immutable git objects with `--rev e9408ce4`, independently
of the checked-out branch. The script enumerates
git-tracked `.rs` files under `crates/kf-*`. It excludes any path component named
`tests`, and, in every remaining file, drops the first line whose text, after
leading whitespace, begins with `#[cfg(test)]`, and everything to EOF. Blank
lines preceding that physical line are retained. It counts physical lines,
not statements or semicolons. Nested block comments, string literals, raw strings
and character literals are distinguished. Retained lines partition into code,
comment-only and blank; a code line with a trailing comment counts as code, with
the inline-comment count recorded separately in JSON.

**MEASURED:** “Generated” in the following table means exactly
`crates/kf-abi/src/generated/`, per the requested path convention. “Outside” is
the remainder, **not a guarantee of human authorship**. For example,
`kf-chan/src/video_generated.rs` contributes 1,353 generated code lines to
“Outside”; generated headers in `kf-abi/src/memory_list_generated.rs`,
`kf-abi/src/sw_runlist_native_generated.rs` and
`kf-host/src/precomp_layout.rs` contribute another 390, 61 and 38 respectively.
The detached generator under `kf-abi/gen` is a separate final row, excluded from
the workspace total; its manifest still names it `kayfabe-abi-gen`.

**MEASURED — requested literal rule:**

| Workspace crate | Code | Comment-only | Generated path code | Outside that path |
|---|---:|---:|---:|---:|
| kf-abi | 54,790 | 22,527 | 36,812 | 17,978 |
| kf-arch | 891 | 2,267 | 0 | 891 |
| kf-broker | 5,201 | 1,708 | 0 | 5,201 |
| kf-census | 3,642 | 633 | 0 | 3,642 |
| kf-chan | 11,900 | 3,808 | 0 | 11,900 |
| kf-chip | 2,545 | 1,526 | 0 | 2,545 |
| kf-core | 1,631 | 558 | 0 | 1,631 |
| kf-crec | 1,593 | 818 | 0 | 1,593 |
| kf-cuda | 6,054 | 2,434 | 0 | 6,054 |
| kf-disp | 16,937 | 3,347 | 0 | 16,937 |
| kf-gop-image | 282 | 150 | 0 | 282 |
| kf-gsp | 3,308 | 3,404 | 0 | 3,308 |
| kf-harness | 8,915 | 925 | 0 | 8,915 |
| kf-host | 3,463 | 1,758 | 0 | 3,463 |
| kf-linux-raw | 4,507 | 4,945 | 0 | 4,507 |
| kf-mem | 9,941 | 2,822 | 0 | 9,941 |
| kf-oprom | 1,165 | 438 | 0 | 1,165 |
| kf-qemu | 22,809 | 5,532 | 0 | 22,809 |
| kf-rawclient | 4,180 | 836 | 0 | 4,180 |
| kf-rm | 16,329 | 10,489 | 0 | 16,329 |
| kf-trace | 153 | 120 | 0 | 153 |
| kf-trap | 3,993 | 1,827 | 0 | 3,993 |
| kf-util | 1,802 | 1,329 | 0 | 1,802 |
| **23 workspace crates** | **186,031** | **74,201** | **36,812** | **149,219** |
| Detached kf-abi/gen, additional (approximate; see below) | ≈2,500 | 445 | 0 | ≈2,500 |

**MEASURED:** Re-running the pinned counter reproduces 4,494 workspace code
lines containing comments. Its physical-line blank count is 13,675. A
whole-text `^\s*` cutoff can also consume preceding blank lines: that convention
reproduces the independent check's 13,414 blanks, a difference of 261 blanks
and no code lines. The published JSON uses the physical-line convention above.

**INFERRED — approximate auxiliary figures:** Treat the detached generator as
about 2.5k code lines and the workspace-plus-generator total as about **188.6k**.
The pinned lexer rederives 2,522 generator code lines and a total of 188,553;
the independent check reports 2,541 and 188,572. The 19-line generator
classification discrepancy remains unresolved, so neither auxiliary code total
is presented as a convention-independent exact result. The JSON publishes this
lexer's full per-file tally rather than silently substituting a different
classifier. These auxiliary differences do not change any of the 23 workspace
code rows or their 186,031 total. **MEASURED:** Under the published counter,
the combined comment-only total is 74,646 and code outside the generated
directory is 151,741. The generated directory has 44,485 physical lines including
comments and blanks. Its largest file, `generated/matrix.rs`, has 31,773 code
lines: version ranges, structure fields, class sets and constants generated by
`tools/drivermatrix/emit_rust.py`. It alone is 17.1% of the 186,031 total.

**INFERRED:** The question's approximate 268k/84k/180k split is not reproducible
as code-only lines under this exact rule at this revision. Counting conventions,
revision and the meaning of “generated ABI” must accompany any comparison.
Adding comments to code is not an implementation-size measurement.

### The literal rule is not the compiled-product boundary

**MEASURED/CODE READ:** The rule includes at least **16,866 code lines** in
separate `src/` test/model files: 7,805 in `kf-disp/src/engine/` tests and models,
4,086 in `kf-mem/src/sim.rs` and `src/sim/`, and 4,975 in the other listed
test/model files. Their parent module declarations are `cfg(test)`; the child
files themselves lack that leading attribute. Examples are
`kf-qemu/src/hostidx_model.rs` (557), `kf-core/src/tests.rs` (1,176) and
`kf-abi/src/vbios/tests.rs` (1,074). These lines are useful tests, not product
instructions and not dead code to delete.

**MEASURED/CODE READ:** Conversely, the rule removes live product code after
interspersed tests. `kf-qemu/src/chan.rs` first hits `cfg(test)` at line 417,
counts only 285 code lines, and drops the other 9,398 physical lines, including
`ChanPlane` and most production channel orchestration. The whole file has 8,033
code lines **including tests**. `kf-core/src/plane.rs` similarly hits a test
module at line 45 before its `HostOps`, `Vmm` and `Plane` definitions. Neither
whole-file count is a valid substitute for a non-test count.

**INFERRED:** Do not subtract 16,866 and call the result the true product size:
the opposite undercount must also be repaired. The literal table answers the
specified counting question; it is unsuitable as a source-size ratchet without
parsing item scopes and following conditional module declarations.

### A second estimate that repairs those two counting artifacts

**MEASURED:** A normal, non-test `cargo check -p kf-qemu --lib` supplied compiler
artifact JSON and dependency files identifying **366 Rust source files in 19
libraries**. An external `syn` parser then removed each complete literal
`cfg(test)` item/method/field by its original-source line span, preserving
production items after inline tests. The same comment/string lexer counted the
remaining original source. `AU1_ast`, `AU1_product_count.py`, the selected-file
list, excluded spans and per-file JSON are in the report artifacts. Four helper
tests cover attribute discrimination and the two inline-test cases that broke
the literal rule.

**MEASURED — compiler-selected library-source estimate:**

| Library | Code after test-item exclusion |
|---|---:|
| kf-abi | 54,295 |
| kf-arch | 891 |
| kf-broker | 5,201 |
| kf-chan | 12,694 |
| kf-chip | 2,482 |
| kf-core | 802 |
| kf-cuda | 5,790 |
| kf-disp | 9,200 |
| kf-gop-image | 32 |
| kf-gsp | 3,308 |
| kf-host | 3,476 |
| kf-linux-raw | 4,690 |
| kf-mem | 5,856 |
| kf-oprom | 1,024 |
| kf-qemu | 29,092 |
| kf-rm | 16,571 |
| kf-trace | 153 |
| kf-trap | 2,741 |
| kf-util | 2,264 |
| **Total** | **160,562** |

**MEASURED/CODE READ:** This estimate has 71,878 comment-only and 12,396 blank
lines; 123,750 code lines are outside the specified generated directory. Its
`chan.rs` contribution is **7,003 code lines** after removing its inline tests,
which explains the largest discrepancy in the QEMU figures. Build scripts,
examples, tool binaries and firmware source are outside this runtime-library
estimate; the mandated inventory above includes applicable build/example `.rs`
files. The four independent validation/tool crates are outside this dependency
closure. The generated directory still contributes 36,812 code lines.

**INFERRED — limits:** This repairs whole-test-module inclusion and early-tail
truncation. It does not evaluate every non-test `cfg` branch, expand every macro,
prove public items reachable, or account for linker elimination. Its unit is
original physical source lines in compiler-selected files, not executable
instructions. Treat it as a better scoped estimate, not a new release gate.

## 3. What the eight requested crates contain

**MEASURED:** The requested eight together account for **93,678** literal code
lines: 50.4% of all workspace code, or 62.8% of code outside the generated path.
They are a useful investigation set, but not the eight largest literal
handwritten buckets: outside-path `kf-abi` and `kf-harness` also rank highly,
and separate tests inflate `kf-disp` and `kf-mem`.

**CODE READ; file counts are MEASURED under the literal rule:**

| Crate | Main size drivers, with concrete files | Interpretation (INFERRED) |
|---|---|---|
| kf-qemu | `display.rs` 4,673; `device.rs` 3,431; `userpump.rs` 2,959; `mem.rs` 2,685; `userrelay.rs` 991; plus the omitted bulk of `chan.rs` | **INFERRED:** Runtime composition and resource lifetimes sit here alongside the frontend ABI. Much of it could serve another VMM. |
| kf-rm | `chanlink.rs` 1,821; `inittables.rs` 1,489; `hostquery.rs` 1,480; `hostfacts.rs` 991; `rmgraph.rs` 852; `sweep.rs` 732; `rmrpc/policy.rs` 681 | **INFERRED:** RM object graph, guest allocation/control admission, asynchronous statements, host-fact derivation and authored initialization replies. These are many distinct wire/control obligations, not one forwarded ioctl loop. |
| kf-disp | `engine.rs` 2,007; `scanout.rs` 1,526; `model.rs` 1,439; `pace.rs` 600; plus 7,805 counted test/model lines | **INFERRED:** Virtual display channels, interlocks and completion semantics; head/window state, scanout addressing/format plans, EDID/caps, color, pacing and broker-credit handling. Large test scope is an accounting artifact, not shipping emulation. |
| kf-chan | `host.rs` 1,378; `video_generated.rs` 1,353; `user_lifecycle.rs` 892; `translated.rs` 863; `user_turn.rs` 725; `dbfast.rs` 645 | **INFERRED:** Host-authored ring publication, method decoding/rewriting, generated video-method address tables, USER relay state machines, completion relays and fast doorbells. `user_turn` is actually shared with QEMU's concrete pump. |
| kf-mem | `batch.rs` 1,977; `vasmgr.rs` 1,302; `apply.rs` 1,101; `cpuwin.rs` 692; plus 4,086 counted simulator lines | **INFERRED:** Mapping transactions, ownership, overlap/granularity handling, commit-on-ack, retaining unchanged mappings, async reconciliation and retargetable CPU windows. The simulator tests transient correctness as well as final state. |
| kf-linux-raw | `mapping_unsafe.rs` 660; `kvm_unsafe.rs` 383; `memtype.rs` 381; `vcpu_unsafe.rs` 345; `chardev_unsafe.rs` 331 | **INFERRED:** Checked OS/FFI primitives and their audit perimeter, mapping/cache geometry, device ioctls, eventfds/epoll, sockets and KVM test rigs. KVM/vCPU wrapper uses outside their implementation files are in `kf-chan/tests/common/mod.rs` and `kf-linux-raw/tests/kvm_vm_discovery.rs`, with no `kf-harness` caller; they are not the QEMU product's hypervisor implementation. Its 4,945 comment-only lines exceed code under this rule. |
| kf-broker | `conn.rs` 2,517; `slots.rs` 539; `ticks.rs` 442; `cursor.rs` 425; `wire.rs` 393; `input.rs` 359 | **INFERRED:** Real connection/reconnect and protocol state, negotiated frame rungs, fd/slot ownership, pacing credits, cursor stream and input policy. Input and cursor operations already have VMM-neutral sinks. |
| kf-cuda | `walk.rs` 1,539; `display.rs` 1,430; `driver_unsafe.rs` 924; `abi.rs` 659; two color examples 432 combined | **INFERRED:** CUDA driver-API glue, walker/report/ack lifecycle, GPU scanout operations, bounded buffers and GPU completion. Oracle/model/synthetic fixtures also live in the library; examples are not product runtime. PTX/CUDA source outside `.rs` is not in these totals. |

**CODE READ:** Per-family variation is concentrated in `kf-chip` and generated
ABI/class/method data, rather than five complete copies of these runtimes.
`kf-chip/src/lib.rs` has five `Family` variants and source-derived per-family
choices; the entire crate is 2,545 literal code lines. This does not prove every
family/die is supported or measured: family intent and hardware qualification
are separate, as [the release ledger](../RELEASE_CLAIMS.md) requires.


## 4. Why the QEMU crate is not just a small QOM shim

**MEASURED/CODE READ:** The C QOM implementation is one translation unit,
`qemu/hw/misc/kf3/kf3.c`, 2,027 physical lines; the three local headers add 329.
There is no second local `.c` device implementation. The following table sorts
the largest Rust files by the literal retained code count, adding `chan.rs`
because the cutoff otherwise hides the largest whole file.

| File under kf-qemu/src | Literal code | Why it is large (CODE READ) |
|---|---:|---|
| display.rs | 4,673 | Register-to-display routing; worker state, latch/acquire/release ordering, boot-to-NVIDIA display handoff, GPU copies, color/scanout plans, VRAM broker rungs, cursor and console synchronization, diagnostics. This is display runtime, not QOM registration. |
| device.rs | 3,431 | Realize validates configuration, opens/derives host resources and starts planes; register drainer, worker/VA-loop coordination, interrupts, RAM publication, reset coordination and teardown. |
| userpump.rs | 2,959 | Concrete USER-ring accessors and retained snapshots, native publication, staged software triggers, source changes and diagnostics. The per-entry state machine lives in `kf-chan::user_turn`; this file supplies the real I/O. |
| mem.rs | 2,685 | Guest-RAM topology and fd-offset translation, placement resolvers, BAR/PRAMIN CPU views, VA-manager composition, deferred overlays and their acknowledgements. |
| userrelay.rs | 991 | Ring-route selection, per-channel relay resources, lifetime/birth state, worker wakeup and cleanup. |
| ffi_unsafe.rs | 739 | Exported C ABI and its structures; handle/buffer checks, property conversion, memory/interrupt/display/reset entrypoints. It is glue, not the majority of the crate. |
| hostidx_model.rs | 557 | Test-only concurrency model, included only under the parent's `cfg(test)`; counted accidentally by the literal rule. |
| userva.rs | 549 | Restricted private command-mapping ownership and exclusion admission; queued requests executed on the VA worker; uncertain cleanup retains authority. |
| readtrace.rs | 411 | Bounded, default-off BAR0 read diagnostics and trace admission. These are required diagnostics, not evidence of a production general-read trap. |
| dispsw.rs | 390 | Authored host display-software twins, mapping leases and bounded resource budgets. |
| chan.rs, cutoff outlier | 285 | Whole file: 9,814 physical / 8,033 code lines including tests. Native twin/group/engine-object birth, schedule/free, parked host acts, translated execution and relay ownership dominate the excluded production portion. |

**MEASURED:** Under the supplementary item-scope count, `chan.rs` is 7,003
non-test source code lines and the crate is 29,092. The next five large files
retain the counts above: their first test section follows their production
code. This makes the runtime-composition explanation stronger, not smaller.
`ffi_unsafe.rs` and `raw_unsafe.rs` together are **1,097 code lines**, just 3.8%
of the library-source estimate for this crate.

**INFERRED:** “Small C QOM device” remains a reasonable description of the bus
shim, not of the whole `kf-qemu` crate. The crate name obscures that it is also
the current device-runtime composition root. Calling its entire size “QEMU API
glue” materially overstates the work a new VMM would need to reproduce.

## 5. Removal, relocation and merge candidates

**MEASURED — evidence method:** Searches used `rg` across the workspace,
manifests/features, tests, scripts, tools and firmware. Identically named items
in `kayfabe-*` were resolved to their separate crate, not treated as v3 callers.
An exported scratch copy of the audited worktree was made under the lane's
temporary target directory; only that copy received removal edits. The audit
worktree's code stayed unchanged. `CONFIRMED` below requires no workspace caller
and successful `cargo check --workspace --all-features --all-targets` with the
item absent in that scratch source tree; it does not mean a public downstream
consumer is impossible. No deletion is proposed by this documentation PR.

| Candidate | Evidence and classification | Recommendation |
|---|---|---|
| `kf-cuda::driver_unsafe::memcpy_h2d_async` | **CONFIRMED** in the scratch check recorded below. The method's own `allow(dead_code)` says it has no caller since P4b. Exact-symbol search finds its definition and a doc reference from the other copy helper, but no call. Removing its 34 physical / 21 code lines leaves the actual CUDA symbol binding intact. | Remove this wrapper in a separate reviewed cleanup, or name a real consumer. Do not remove `memcpy_d2h_async`: four live calls in `kf-cuda/src/display.rs` disprove that broader deletion. |
| `kf-cuda::selftest` module API | **CONFIRMED** in the scratch check recorded below. `bring_up_and_prove` / `probe_after_sandbox` have no v3 workspace caller. Actual isolate callers explicitly use **`kayfabe_cuda`**. The scratch removes the v3 module declaration, not the frozen implementation. The module is 202 literal code lines / 305 physical lines. | Retire or relocate this copied isolate-era API after checking any external consumers; do not touch the frozen self-test. |
| `kf-cuda::synth` and `diffmodel` | PLAUSIBLE relocation, not dead. `kf-harness/src/tables.rs` uses `synth`; gate 9 uses `diffmodel`; the memory simulator and `vasmgr` tests also use it. `synth` is 190 literal code lines, `diffmodel` 244. | Put fixture construction in a test-support boundary if worthwhile. Keep the independent walker oracle; do not merge it into the implementation it checks merely to save lines. |
| `kf-linux-raw` KVM/vCPU rig | PLAUSIBLE relocation, not dead. `kf-chan/tests/common/mod.rs` and `kf-linux-raw/tests/kvm_vm_discovery.rs` use `Kvm`, `KvmVm`, `KvmMemslot`, `KvmVcpu`. Together the two implementation files contribute 728 literal code lines. | A future test-support feature/crate could isolate these APIs. Do not remove their adversarial doorbell tests; feature isolation needs its own gate review. |
| Scalar wire readers across `kf-rm/src/hostfacts.rs::le16/le32`, `kf-oprom/src/rom.rs::le16/le32` and `kf-census/src/capture.rs::le32` (line 358) | PLAUSIBLE merge of duplicated little-endian reading logic, with different bounds contracts. `hostfacts.rs::le32` computes unchecked `at + 4` before `get`; `rom.rs::le32` uses `checked_add`; the census helper indexes four bytes directly. `kf-abi/src/wire.rs` has a further error-returning variant. These are small helpers, not interchangeable parser contracts. | At most a small shared bounded-reader utility; expected savings are tens of lines, not thousands. Preserve short-input/error behavior. |
| `kf-qemu/src/gpuuid.rs` vs `kf-rm/src/gpuuid.rs`, and `userpump` vs `kf-chan::user_turn` | PLAUSIBLE duplication hypotheses rejected by CODE READ: the QEMU UUID wrapper delegates to RM's implementation, and the pump uses the shared turn machine. | Keep policy and concrete adapter roles; merging crates would not remove an independent implementation. |
| `kf-qemu/src/chan.rs::EngineObject::class` | PLAUSIBLE small dead-field candidate: a local `allow(dead_code)` explicitly reserves it for a forthcoming binding lookup. Not removed/compiled in scratch. | Recheck all field reads and intended binding work before deleting; no bulk saving claimed. |

**MEASURED:** The complete exported scratch tree passed the all-feature,
all-target workspace check with both removals together in 59.52 seconds.
Retiring the unreferenced module file as well as its declaration and the upload
wrapper would remove **224 code / 340 physical lines**: 0.12% of the literal
workspace code or 0.14% of the library-source estimate. These two small
candidates cannot justify a repository-wide “mostly dead” claim.
Static caller searches cannot prove dynamic
feature demand, guest reachability, external API use or behavior on hardware.

### Frozen duplication is a different bucket

**MEASURED:** Byte-for-byte comparison of same-relative-path `src/*.rs` files
in the live and frozen crate pairs found:

| Pair | Identical files | Physical lines, including tests/comments | Examples |
|---|---:|---:|---|
| kf-linux-raw / kayfabe-linux-raw | 10 | 2,488 | memtype.rs, cache.rs, geometry.rs, bounds.rs |
| kf-abi / kayfabe-abi | 14 | 5,488 | fbinfo.rs, fmbpromote.rs, transcribed.rs, regaccessmap.rs |
| kf-util / kayfabe-util | 6 | 3,135 | trapwitness.rs, coverage.rs, lockwitness.rs, interval_map.rs |

**CODE READ:** The root workspace retains ten `kayfabe-*` crates for the frozen
raw grader and its dependency closure; root `AGENTS.md` forbids introducing a
`kf-*` dependency on them. The counts above show maintenance duplication in the
repository, not an extra 11,111 lines inside the `kf-*` total. Some copies can
diverge deliberately. `kf-cuda` / `kayfabe-cuda` has no byte-identical complete
same-path Rust file in this comparison.

**INFERRED — PLAUSIBLE future retirement, not confirmed removability:** Finish
and qualify the replacement raw grader before retiring its
frozen dependency closure. Sharing old and new types to save source now could
reconnect the planes the owner explicitly separated. No deletion of the frozen
tree is justified by this audit.

## 6. What can actually be compared with Guesthouse

**INFERRED — external-project reports, public sources read in the initial
2026-11-08 audit; not independently network-verified by the fact-checker:** Guesthouse's
[README](https://github.com/xnixy/guesthouse) describes a Windows GPU-PV/VMBus
route, a Linux driver-call-forwarding route, a separate daemon, desktop viewer
and installer. Its published QEMU-patch figures are about **900 Windows lines**
plus **4,800 Linux lines**. These are author-reported patch figures, with no
matching code/comment/test convention. The README and
[architecture](https://github.com/xnixy/guesthouse/blob/main/ARCHITECTURE.md)
read for this audit did not supply a comparable total non-test Rust LOC figure.
No Guesthouse implementation code or device values were imported or used.

| Scope | Kayfabe (CODE READ) | Guesthouse (INFERRED from public documentation reports) |
|---|---|---|
| Guest protocol | Emulated GPU PCI/BAR/GSP/RM; stock physical-device driver path | Windows Hyper-V GPU-PV/VMBus; Linux driver-call forwarding |
| Display plane | NVDisplay methods, interlocks, head/window model, EDID, scanout and broker | Viewer/frame transport and Windows indirect display driver; desktop display exists, not the same emulated NVDisplay contract |
| Boot display | Authored PCI option ROM and embedded GOP driver | Basic-display path is documented; no equivalent authored GOP-ROM implementation established by the docs read |
| GSP/RM | Fake GSP boot/queues and GSP-side RM answers, plus authored host verbs | Vendor backend talks to host RM; equivalent guest GSP/RM emulation is not established by the docs read |
| Families/driver versions | Five family variants, generated per-tag ABI matrix; per-die host facts | Generation and driver-branch profiles; docs describe one physically exercised GPU and additional experimental generation profiles |
| Other included scope | GPU gates, raw-client replacement, census/replay tools and source-built boot firmware | Desktop application, installer and display driver also add scope; do not assume the whole competitor tree is merely GPU core |

**MEASURED — kayfabe-side accounting, not a measured cross-project gap:**

| Disjoint literal-source bucket | Code lines | Share of 186,031 |
|---|---:|---:|
| Generated ABI directory | 36,812 | 19.8% |
| Four validation/offline crates: census, crec, harness, rawclient | 18,330 | 9.9% |
| Display/broker/ROM/image crates | 23,585 | 12.7% |
| RM/GSP/chip/architecture vocabulary | 23,073 | 12.4% |
| All remaining code | 84,231 | 45.3% |

**CODE READ — inconsistency for a separate follow-up:** `kf-trace` is
deliberately **not** in the non-product tool bucket. `ARCHITECTURE.md` line 101
groups it with `kf-crec` as “never linked into the product”, but
`crates/kf-rm/Cargo.toml` lists a normal dependency on it and
`crates/kf-rm/src/rmrpc/mod.rs:226` imports `kf_trace::FaultTag`. This audit
records the inconsistency; it does not edit `ARCHITECTURE.md`. The four other tool
crates
above are absent from `kf-qemu`'s normal dependency closure. Test-file inflation
from section 2 overlaps the functional buckets and must not be subtracted twice.

**INFERRED — numerical limit on the requested attribution:** These buckets
identify **101,800 lines (54.7%)** of generated-data, validation and named
emulation/display scope. They do **not** prove that all 101,800 lines are an
increment over Guesthouse, which has overlapping display and backend work.
The remaining 84,231 lines are not an estimate of waste. The two scratch
removals identify only a small lower bound on removable source; the upper bound
on accidental source remains unmeasured. A percentage of the **gap** cannot be
calculated without a comparable Guesthouse total: if that total is `G`, the gap
is `186031 - G`, and neither `G` nor its scope allocation was established here.
Assigning a percentage would be invented evidence. Even the author-reported
roughly 5,700-line patch is larger than kayfabe's 2,027-line C file; it says
nothing about either complete implementation's relative size.

## 7. Where QEMU enters, exactly

**CODE READ:** QEMU's actual APIs are in `qemu/hw/misc/kf3/kf3.c`; Rust is linked
as a static library exporting `kf3_*` functions from
`crates/kf-qemu/src/ffi_unsafe.rs`. Rust's raw-memory/callback boundary is
`raw_unsafe.rs`. Its callbacks carry opaque frontend context and plain C values;
the Rust runtime does not import QEMU's `PCIDevice` or `MemoryRegion` layouts or
link against a QEMU Rust crate. That is an existing reuse opportunity, despite
the crate's name.

| Area | QEMU side and Rust seam (CODE READ) | Consequence for a second frontend (INFERRED) |
|---|---|---|
| PCI/QOM and identity | `TypeInfo`, `PCIDeviceClass`, properties, `kf3_dev_realize/exit`; Rust `Config`, `kf3_realize`, `kf3_identity`, `kf3_config_word` | Supply PCI configuration/BAR sizing, guest-visible identity and VM/device identity inputs; reuse host discovery and GPU planes. |
| BAR0 | `kf3_bar0_build`, ROM-device shadows, RAM-device pieces, lockless `MemoryRegionOps`, `kf3_shadow_attach/seal`, `kf3_bar0_write/read` | Install direct-read/write-trapped shadow mappings, direct timer/usermode pieces and the family-scoped read holes. Do not replace it with generic read/write MMIO dispatch. |
| BAR1/BAR2/PRAMIN | `kf3_bar_build`, `memory_region_init_ram_device_ptr`; Rust `kf3_bar_ram`, `MemPlane`, `CpuWindow`, `ViewOps`, `GuestWindow` | Stable process-local virtual windows are registered with KVM; Rust retargets backing with `MAP_FIXED`. Preserve scratch backing and lifetime, not just a raw GPA range. RM's BAR2 is PCI BAR3. |
| Hopper+ BAR1 doorbell aliases | `kf3_bar1_overlay`, FIFO bottom half, `MemoryRegion` aliases; `OverlayHook` and `kf3_bar1_overlay_done` | Changes must be asynchronous, sequence-numbered and acknowledged; do not clear the relevant invalidate before the overlay is live. |
| Guest RAM/DMA | `MemoryListener` add/del, `kf3_is_guest_ram`, backend fd/file offset into `kf3_ram_add/del`; Rust `RamMap`, `RawRegion`, `BackendFd`, `MemoryListRam` | Register writable real RAM only; preserve topology generations, revoke before backing disappears, and supply the common memfd plus per-block offsets. |
| Doorbell acceleration | C `kf3_ioeventfd` uses `KVM_IOEVENTFD` with DATAMATCH; listener tracks doorbell sites; Rust `IoeventfdHook` implements `kf_chan::dbfast::Ioeventfd` | Preserve the existing token dispatch semantics and async registration. A refusal retains the trapped path; never add synchronous GPU work to a vCPU. |
| Interrupts | Rust owns per-vector eventfds (`kf3_irq_fd`); C MSI-X route use/release callbacks install/remove KVM irqfds | Implement MSI-X programming, mask/unmask/pending behavior and loss-free eventfd delivery. Diagnostic fallback drains on the main loop and calls `msix_notify`; there is no plain-MSI-only frontend here. |
| Reset | QEMU Resettable hold/exit, `current_cpu`, `RESET_TYPE_WAKEUP`; `kf3_device_reset/reset_wait`, Rust `reset`, `scrub`, `memreset` | Pass reset cause explicitly. Reboot, guest-issued bus reset and S3 wake have different scrub/lifetime rules. A main-loop wait with vCPUs paused is not permission to wait on a running vCPU. |
| Display/console/input | `graphic_console_init`, `dpy_*`, pixman surface/cursor wrappers, fd/timer callbacks and QEMU input/power routing; `QemuSink`, `BrokerHooks`, display FFI | Broker policy can be reused. Implement optional UI/input/cursor sinks; a headless frontend need not emulate QEMU's console API. |
| Migration/snapshot | No `VMStateDescription`, `vmsd` assignment or device save/load implementation in the overlay/Rust ABI; `nomigrate` RAM initializers are present | No implemented device migration contract was found. A new frontend must explicitly refuse unsupported snapshots/migration; serializing native handles is not a substitute. |
| Chardev | No `CharBackend`, QEMU chardev registration or chardev FFI in this overlay | No GPU chardev port needs translating. The display broker is a Unix-socket/fd protocol; Linux `CharDevice` wraps host device ioctls and is unrelated to QEMU chardev. |

**CODE READ:** The existing C realize path refuses configurations that pin these
retargetable RAM-device windows, using `ram_block_discard_require(true)`.
Its listener is registered against `address_space_memory`.
`RamMap::dma_to_file_range` presently resolves DMA as GPA, not through a built
guest-IOMMU translation service. Rust comments say realize refuses a vIOMMU, but
the searched C/Rust device sources contain no `pci_device_iommu_address_space`
check or other identified explicit vIOMMU admission check. **INFERRED/OPEN:**
Do not treat those comments as verified enforcement; a focused follow-up should
prove refusal or add the missing check. A new frontend must reject unsupported
IOVAs explicitly. No guest-IOMMU behavior was tested in this lane.

### Dependency and symbol checks

**CODE READ — sources of the portability propositions:** Only `kf-broker` is
explicitly labelled VMM-agnostic in `ARCHITECTURE.md`'s crate table. The four
propositions do not all originate in that table:

| Crate | Claim source and scope |
|---|---|
| kf-broker | CODE READ: `ARCHITECTURE.md`'s crate row and `crates/kf-broker/Cargo.toml` explicitly call it VMM-agnostic. |
| kf-trap | CODE READ: `crates/kf-trap/src/vmm.rs` opens with the VMM-agnostic seam claim; its interface comments are the source, not the architecture crate row. |
| kf-core | CODE READ: owner ruling AO.1 and the version-axis tables in `THE_ARCHITECTURE_v3.md` / `THE_DESIGN.md` describe a VMM-agnostic core; source `kf-core/src/lib.rs` describes composition around `HostOps`. This is design intent to check against dependencies, not a working second adapter. |
| kf-mem | INFERRED: this audit tests QEMU independence from its dependency closure and `MapTarget` / `ViewOps` boundaries. The architecture crate row describes the memory plane without explicitly calling it VMM-agnostic. |

**MEASURED:** Ran, offline and locked:

```sh
cargo tree -p kf-core -p kf-trap -p kf-mem -p kf-broker \
  --offline --locked --all-features -e normal,build,dev
rg -n -i '\bqemu|\bQOM|MemoryRegion|msix_notify|pci_device|chardev' \
  crates/kf-trap/src crates/kf-core/src crates/kf-mem/src crates/kf-broker/src
```

**MEASURED/CODE READ:** No `kf-qemu` dependency appears in those four closures,
including build/dev edges. `kf-core` depends normally on `kf-trap` and `kf-chip`;
`kf-trap` on `kf-chip` (loom is dev-only). `kf-mem` reaches `kf-host`, `kf-cuda`,
`kf-trap`, `kf-chip` and the Linux raw adapter; `kf-broker` reaches the Linux raw
adapter. They are QEMU-independent, not OS/GPU independent. Search hits are
comments, test text and, in broker connection errors, QEMU-specific wording;
they do not call QEMU APIs. Broker cursor coordinate policy also documents the
current frontend's axis scaling, so UI semantics still need review in a port.

**CODE READ:** The other GPU-model crates (`kf-abi`, `kf-arch`, `kf-chip`,
`kf-gsp`, `kf-rm`, `kf-chan`, `kf-disp`), host/CUDA/OS adapters and ROM construction
likewise have no normal dependency on `kf-qemu`. GPU/RM and Linux/KVM ties remain
where named. `kf-core::Vmm` is a **struct containing the shared wake word**, not
a hypervisor trait. `HostOps` is GPU-plane host execution, not PCI/device
lifecycle. The hypervisor-facing traits are in `kf-trap::vmm`.

## 8. Minimal frontend surface: existing pieces and the missing joins

**CODE READ:** `kf-trap/src/vmm.rs` already defines `GpuDevice`, `MmioOutcome`
and `VmmOps`. The latter has guest read/write, install/remove memslot, raise IRQ
and signal worker/drainer methods. `memmap::install` consumes it. Searching
implementations finds `RecordingVmm` in `kf-trap/tests/memory_map.rs`; no product
implementation was found. QEMU obtains the same map through `kf3_memory_map`
and builds it in C. An unused interface shape is not evidence that all runtime
composition is already abstracted.

| Required contract (INFERRED minimal port boundary) | Existing implementation/trait (CODE READ) | What remains frontend-specific |
|---|---|---|
| BAR0 write delivery and bounded outcome | `GpuDevice::mmio_write`, `MmioOutcome`; production `Device::bar0_write` and FFI | Wire the production device into that shape; preserve trap classification and nonblocking wake/inline-ring outcomes. |
| Direct BAR mapping and retargetable backing | `VmmOps::install_memslot/remove_memslot`, `MemoryMap`; `ViewOps`, `CpuWindow`, `GuestWindow` | Stable window lifetime, read-only slots, PCI relocation/decode updates, and async BAR1 overlay request/ack. `HostMapping(pub u64)` is only a public numeric token today, not a production bounded-memory authority by itself. |
| Guest RAM reads/writes and DMA translation | `kf_gsp::ram::GuestRam`; `kf_rm::memory_list::GuestRamAuthority`; QEMU `RamMap` and topology generations | A frontend-owned backing registration/revocation API and fd/file-offset translation. The DMA-address-to-RAM seam must refuse unsupported IOVAs. |
| Interrupt injection | `VmmOps::raise_irq`; per-vector Rust eventfds | Bind eventfds to native MSI-X routes and replay pending work across mask changes; no blocking callbacks from workers. |
| Fast doorbell transport | `dbfast::Ioeventfd` and `Sink` | Register/deregister each token at every mapped doorbell site and preserve its lifetime/ack rules. A generic socket write is not equivalent. |
| Reset and lifetime | Rust `reset::Owners`, gate/stages, `scrub::Event` and device reset methods | An explicit reset-cause value replacing QEMU's `current_cpu`/reset-type test; topology/device shutdown must quiesce users before releasing memory. |
| Optional display, input and event loop | `kf_broker::Host`, `InputSink`, `CursorSink`, `Link`, frame/cursor shares | Timers/fd watches, input/power/resize operations and optional console presentation; the GPU display engine and broker protocol do not need rewriting. |
| PCI registration and startup/teardown | Rust `Config`/identity/map description; existing C ABI | PCI capabilities, option ROM exposure, VM identity, backend resource ownership and migration refusal. No single existing trait covers these together. |

**CODE READ:** The actual trap policy has narrower exceptions than the slogan
“BAR0 writes only”: `memmap::holes_for` names side-effectful read pages on
Hopper/Blackwell; `bar1db` handles runtime BAR1 usermode-doorbell aliases; owner
ruling A allows the bounded PRAMIN window move. `MmioOutcome::Barrier` also
exists, but its presence is not authorization to block the vCPU waiting for
mapping or GPU work. A port must preserve the current exceptions rather than
introduce general BAR1/BAR2 traps or synchronous work.

## 9. Candidate frontends and effort bounds

**INFERRED — estimate convention:** The following are engineering estimates for
one experienced maintainer, for an initial Linux/KVM frontend using the existing
GPU runtime and broker, with Linux guest boot, reset and correctness validation.
They count new/changed handwritten adapter and integration code, excluding
wholesale moves, vendored libraries, generated ABI data and new test code.
Tests are estimated separately. They exclude upstream acceptance, broad
family/driver qualification, live migration, arm64 qualification and Windows/UI
parity. There is no measured implementation throughput behind the week ranges.
Allow a shared **1–2 weeks / 300–800 new lines** for the runtime hooks and bounded
backing join if done as a prerequisite; moving existing code is additional diff
volume, not a new implementation of the GPU planes.

| Candidate | External source reports (INFERRED; public links) | Required work and constraints at risk (INFERRED) | Initial adapter estimate (INFERRED) |
|---|---|---|---|
| Cloud Hypervisor, in-process | Its [memory documentation](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/memory.md) exposes shared mappings and memory zones. Its [vfio-user HOWTO](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/vfio-user.md) separately documents an experimental out-of-process PCI route with virtio-mem/IOMMU limitations. | Implement custom PCI registration/configuration, MSI-X and the slot/readonly/overlay hooks against its device and interrupt interfaces. Choose one shared backing and map PCI-hole segments with file offsets; do not silently admit multiple memory-zone fds. Keep host work out of bus callbacks and preserve doorbell eventfds. Existing `vmm.rs` comments are a historical trait comparison, not a build against current Cloud Hypervisor. | 2–4k adapter/integration lines, 1–3k tests, **3–6 weeks**, plus the common prerequisite. Best first in-process candidate. |
| crosvm | Its [architecture](https://crosvm.dev/book/architecture/overview.html) describes a sandboxed process per device; [advanced usage](https://crosvm.dev/book/running_crosvm/advanced_usage.html#multiprocess-mode) documents that default and the single-process test option. Its [device inventory](https://crosvm.dev/book/devices/index.html) includes PCI and virtio devices. | Add a custom PCI device and MSI-X/slot path. Decide whether kayfabe lives in the main process or one device process; a device-process port needs backing fd, slot-update and interrupt control across that boundary. Avoid a synchronous proxy round trip on every trap; main-process `MAP_FIXED` cannot retarget another process's mapping. One-memfd and GPU-driver resource access need explicit integration. | 3–6k adapter/integration lines, 2–4k tests, **5–9 weeks**, plus the prerequisite. Sandbox/process integration is a large uncertainty; a single-process experiment is not production sandbox parity. |
| Firecracker | Current [getting-started documentation](https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md) and [kernel policy](https://github.com/firecracker-microvm/firecracker/blob/main/docs/kernel-policy.md#booting-with-pci) describe optional PCI virtio transport. The kernel-policy page also retains an older contradictory “no PCI devices” note. Its [design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) describes synchronous vCPU MMIO and syscall filtering. | PCI infrastructure exists; a general custom NVIDIA PCI GPU frontend is not established by those docs. Add that device/config path, readonly/direct RAM-device slots, MSI-X/eventfds, shared guest backing, reset handling and resource/syscall integration. Stock virtio queues are not a replacement for the stock NVIDIA driver's BAR/GSP interface. Keep GPU work off synchronous MMIO; validate shared-memory/balloon/snapshot restrictions. UEFI/GOP and Windows boot are separate work, not implied. | 5–10k adapter/integration lines, 3–6k tests, **6–12 weeks**, plus the prerequisite. Higher integration risk; not a claim that the PCI bus must be written from scratch. |
| vfio-user / out-of-process | The [protocol](https://www.qemu.org/docs/master/interop/vfio-user.html) defines fd-backed DMA maps, sparse region mmap, IRQ eventfds, optional region I/O fds, reset, and pipelined/no-reply commands. These are protocol capabilities, not a claim that every client implements them. | Build a server/lifecycle adapter, validate RAM ownership/FD offsets, and prove the retargetable-window contract in section 10. BAR0 read shadows plus trapped writes require a suitable client mapping mode; BAR1 doorbell aliases and acknowledgements need dynamic semantics. An ordinary socket round trip for every BAR write threatens no-blocking and latency; trap fallback for BAR1/2 violates the design. | 3–6k server/integration lines + 1–4k client extensions if needed, 2–5k tests, **6–12 weeks after the mapping feasibility proof**. If backing cannot be exported safely, this estimate is invalid and a separate design is required. |

**INFERRED:** None of the estimates is a promise of a supported frontend.
For any candidate, the first falsifying experiment is GPU-free: instantiate its
actual PCI/memory API against a recording device and prove direct reads,
write-only exits, BAR relocation, overlay acknowledgement, fd-offset translation
and MSI-X masks. Hardware verification remains mandatory before compatibility
claims, but is outside this lane and was not attempted.

## 10. Why the process boundary is a separate problem

**CODE READ:** The current in-process mechanism obtains one stable virtual
address from `kf3_bar_ram` and registers it with the VMM. `ViewOps` and
`GuestWindow` change backing at that virtual address using checked `MAP_FIXED`
placements. Unmapped guest pages still have scratch backing. The host aperture
view and release cookie remain owned until the relevant lifetime ends. The
ordinary BAR1/BAR2 mappings are therefore not described accurately as thousands
of new QEMU memslots; runtime doorbell aliases are a separate, acknowledged
subregion update.

**INFERRED, applying [mmap's calling-process address-space semantics](https://man7.org/linux/man-pages/man2/mmap.2.html):**
Passing an fd does not cause subsequent `MAP_FIXED` replacements in the server
to alter the client's virtual mapping. A server needs a stable, safely
retargetable exported backing object, or client-assisted remaps with clear
ownership and acknowledgements. **INFERRED, from the
[vfio-user protocol](https://www.qemu.org/docs/master/interop/vfio-user.html):** Sparse mmap and multiple-fd capabilities
describe regions; their existence alone does not prove this runtime retargeting
operation. The protocol's optional region I/O-fd and no-reply mechanisms likewise
need client support before they can keep a vCPU from waiting for socket replies.

**CODE READ/INFERRED:** [The older vfio-user research](V3_VFIO_USER_FRONTEND.md)
already flags mmap-area churn and suggests several alternatives. Its dated
measurements and proposed options are not a proof for today's stable-window
implementation or today's protocol. Reuse that question, not an assumed answer.
A kernel/mdev helper would introduce another trust and maintenance boundary;
this audit neither requires nor authorizes building one.

## 11. Small preservation work now; extraction when a second frontend exists

**INFERRED — recommendation:** Do not redesign the GPU planes or split the
entire runtime now. Keep three changes small and independently reviewable:

1. **Make the frontend hooks a named contract.** Around
   `kf-qemu/src/raw_unsafe.rs`, `ffi_unsafe.rs` and `device.rs`, collect the
   already-existing overlay request/ack, ioeventfd registration, RAM
   registration/revocation and reset-cause inputs. Use bounded backing handles
   with checked lifetime/offset operations, not a public integer cast to a host
   address. An interface-only follow-up should aim at hundreds of new lines,
   reusing the existing callbacks and `ViewOps`.
2. **Exercise a real production join.** Have QEMU's adapter, or a GPU-free
   recording adapter around the same production entrypoints, implement the
   relevant `kf-trap::vmm` operations. Cover BAR relocation/read-only mappings,
   topology revocation, masked IRQs and delayed overlay acknowledgements. Merely
   adding a second test-only implementation of an otherwise bypassed trait
   would not establish portability. Existing `InputSink`, `CursorSink`,
   broker `Host` and `dbfast::Ioeventfd` are examples to retain.
3. **Keep common runtime logic independent of frontend types.** Future work in
   `device.rs`, `chan.rs`, `mem.rs`, `display.rs`, `reset.rs`, and USER-ring
   modules should use those hooks. Once a second in-process frontend is chosen,
   move the safe composition into a shared library without changing the GPU
   contracts. Raw memory stays within the approved `*_unsafe.rs` perimeter;
   adding an unsafe crate would require a separate owner rule change.

**INFERRED — verdict:** Another in-process Linux/KVM VMM can be integrated later
without redesigning GSP/RM, channel kinds, the GPU walker or the display broker.
The work is a PCI/mapping/lifecycle adapter plus cleanup of today's composition
boundary, not a trait implementation of a few dozen lines. Cloud Hypervisor is
the lowest-risk first candidate on the evidence read. Out-of-process attachment
and vIOMMU support remain separate proof obligations. No code-size argument
justifies dropping safety checks, test oracles, family coverage or diagnostics.

## 12. Validation and limits

**MEASURED:** The initial-audit validation below belongs to the fixed source
revision `e9408ce4`; the later fact-check merge validation is recorded separately.

**MEASURED:** The counting tool's baseline seven tests passed before expansion;
its final ten tests pass. Three deliberate counter defects are killed by at
least two tests each: keeping the test tail (2), counting comments as code (4),
and including `tests/` directories (2). These are audit-tool mutants outside
the repository, not new product gates or modifications to existing pass rules.
The AST helper's four tests pass; its ignore-test-attribute mutant is killed by
3 tests and its item-to-EOF truncation mutant by 2.

**MEASURED:** Before adding this document, the four documentation-gate fixture
suites ran **202 tests, all passing**. The baseline
`cargo check --offline --locked --workspace --all-features --all-targets -j2`
passed in 4m33s. Check compiles tests; it does not run them or establish hardware
behavior. The lane's own target directory, two build jobs, niceness 19 and the
shared local cargo lock were used, with more than 6 GiB free before each build.

**MEASURED:** After adding the document, the same **202 documentation-gate
tests passed**. Documentation freshness checks 215 documents, with 0 existing
debt and 0 new findings (214 documents before this addition). Release-claims
validation reports 133 claims, 0 errors and 3 pre-existing warnings; private
reference scanning is clean; GPU UUID scanning reports 179 baselined files,
0 new and 0 stale findings. No gate baseline was changed. The complete scratch
removal check passed with the working directory and both removals explicitly
verified; the product worktree's code and test suite are unchanged.

**MEASURED — fact-check follow-up after merging `8d495e90`:** The pinned
counter reproduces all 469 original per-file metrics, now published with source
hashes. Its tests increased from 10 to 12, explicitly covering tab-indented
attributes and preservation of the blank line before the cutoff; the three
counter mutants are killed by 4, 4 and 2 tests respectively. The column-zero
variant reproduces 186,624 workspace code lines and 17,178 for `kf-disp`,
confirming why the leading-whitespace wording matters. The four documentation
fixture suites pass 203 tests on the merged tree. Freshness checks 216 documents
with 0 debt and 0 new findings; release-claims validation reports 55 claims,
0 errors and 0 warnings; private-reference and UUID gates remain clean.
The merge imports the reviewed `next` changes; this audit's own diff against
that base contains only this document and the two JSON inventories. No product
build, GPU or VM was run in this documentation follow-up.

**INFERRED — limits:** This is a bounded source audit, not an exhaustive
reachability proof or a performance/security qualification. It does not measure
GPU compatibility, live migration, a second VMM, binary size or guest demand for
every control. Guesthouse total LOC and a numeric scope-versus-waste fraction of
the cross-project gap remain unverified; no code or device constants were taken
from that project. Public documentation may change after the retrieval date.
