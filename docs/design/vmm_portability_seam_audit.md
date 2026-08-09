# ★★ Does the code actually admit a SECOND VMM? — a counted seam audit

**Date:** 2026-08-09. **Method:** read-only source enumeration. No build, no boot. Every number
below is a count over files named in place, and the commands that produced them are given so a
reader can re-run them rather than trust them.

⚠ **Source revision: every count and every line number is at commit `6f0077a`** (CLAUDE.md's rule
— a claim without its revision is not a claim). ⊘ **Not the working tree**, which carried another
agent's uncommitted edits while this was written; re-derive with
`git show 6f0077a:<path>` if the numbers do not reproduce.

> ### ★★ And the drift this audit predicts HAPPENED DURING THE AUDIT — measured, not rhetorical
>
> Seam 5 argues that VMM-neutral composition accumulates inside `kayfabe-qemu-raw` because no gate
> watches for it. While §2 was being written, `crates/kayfabe-qemu-raw/src/shim.rs` grew from
> **3 620** lines at `6f0077a` to **3 767** in the working tree (`git diff --stat`, 2026-08-09:
> `145 insertions(+), 6 deletions(-)`, plus two lines from a second hunk). Its hunk headers are
> at `:224`, `:936`, `:961`, `:2623` and `:2654` — ⇒ **141 of the ~147 new lines landed at or
> after line 668, i.e. inside the VMM-neutral region seam 5 measures.** In one session. By an
> agent doing correct, unrelated, necessary work.
>
> ⊘ That is not a criticism of the change; it is the **only** place that code can currently go.
> It is the evidence that seam 5's debt is **not static** and that a review-based countermeasure
> would not have caught it — which is the argument for the line-count ratchet, made by
> observation rather than by prediction.

**Why this exists.** `vmm_integration_and_support_matrix.md` (owner, 2026-08-09) rules that
QEMU and Cloud Hypervisor are treated identically. The tree contains a **written portability
contract** —

> *"A second hypervisor backend must cost exactly one adapter crate: no trait change, no core
> change."* — `crates/kayfabe-vmm/src/lib.rs:17-18`

— and this project has twice measured a defect of exactly the shape that claim can hide: an
*"unhooked IMPLEMENTATION"* a second architecture could not be added alongside (task #121), and
[[a-table-does-not-decide-behaviour]]. So the contract was tested rather than quoted.

---

## 0. THE ANSWER, before the evidence

**It is not a fiction, and it is not one adapter crate either.**

| claim | verdict |
|---|---|
| *"QEMU wearing an abstraction's name"* | ⊘ **REFUTED.** Two real `Vmm` impls exist over **different substrates**, and the second one was built deliberately non-QEMU for exactly this reason. |
| *"no trait change, no core change"* | ✅ **HOLDS**, and is mechanically gated. |
| *"exactly ONE adapter crate"* | ⊘ **REFUTED, and it was already refuted once by our own second adapter** — which duplicated the memory plane (seam 3) and left 1 682 lines of VMM-neutral code inside a vendor-named crate (seam 4). |
| overall shape | ★ **ADDITIVE — after an extraction that has not been done and that nothing forces.** The extraction is measured at **6 identifier sites** (counted 2026-08-09 over `crates/kayfabe-qemu-raw/src/shim.rs`; seam 5). One question (seam 11) is genuinely unknown and could turn the memory plane into a re-derivation rather than a port. |

**The honest one-sentence answer** (§2, counted 2026-08-09)**:** the *port* admits a second VMM and has proved it — `crates/kayfabe-vmm-kvm/src/lib.rs:1490` is a second real impl over a different substrate; the
*composition root* does not, because it lives inside the QEMU FFI crate — and the reason that is
survivable rather than fatal is that it is coupled to QEMU on **6 lines out of 2 953**, which is
an extraction, not a rewrite.

---

## 1. ★★★ FIRST, TWO REFUTATIONS OF THE AUDIT'S OWN PREMISE

Recorded first because both are instrument failures, and [[suspect-the-instrument-first]] is this
project's most-vindicated rule.

### 1.1 ⊘ The "zero design presence" measurement was a BRE/ERE bug (re-run 2026-08-09)

The premise handed to this audit was: *"MEASURED: `grep -rli 'cloud.?hypervisor' docs/` returns
ZERO files."* Re-run on 2026-08-09 against `docs/`, both ways:

```console
$ grep -rli  'cloud.?hypervisor' docs/ ; echo "exit=$?"      # exit=1, no output
$ grep -rliE 'cloud.?hypervisor' docs/ ; echo "exit=$?"      # exit=0, FIVE files
docs/design/host_execution_plane.md
docs/design/portability_arm64.md
docs/design/l1_os_shell.md
docs/design/testing_doctrine.md
docs/design/l2_qemu_adapter.md
```

Both commands were run on 2026-08-09 against `docs/`. `grep` without `-E` is a **basic** regular expression, in which `?` is a literal character and
`.` is any character — so the pattern searched for the string `cloud?hypervisor` with one
character in place of the `?`. It could not have matched. This is the same shape as the gate
that matched `libc` inside `libcuda` and was permanently red for three rungs: **a zero from an
instrument that could not have returned non-zero is not a measurement.**

★ The correct check is a fixed alternation, not a wildcard:
`grep -rliE 'cloud[- ]?hypervisor|rust-vmm'`. Across `docs/`, `crates/` and `ARCHITECTURE.md`
that returns **ten** files.

### 1.2 ⊘ "`l2_qemu_adapter.md` is QEMU-shaped throughout" — true, and it is the wrong file to look in

It *is* QEMU-shaped throughout, and that is correct: it is the **QEMU adapter's** design doc, and
a document named for one backend describing that backend is not evidence of a leak. The
portability claim lives one layer down, in `crates/kayfabe-vmm/`, and the enforcement lives in
`.github/workflows/ci.yml`. Neither was in the premise's search path.

★ The generalisable error: **the absence of vendor B in vendor A's design doc is not evidence
about the abstraction.** It is evidence about the filename.

---

## 2. The numbered seam list

For each: what a Cloud Hypervisor adapter would have to touch, with `file:line`, and the verdict
**ADDITIVE** (write new code beside) / **EXTRACT** (existing code must move before it is
reachable) / **RE-DERIVE** (must be rebuilt for the new backend) / **DELETE** (CH needs less).

### Seam 1 — the `Vmm` port itself → ✅ **ADDITIVE, zero change**

`crates/kayfabe-vmm/src/lib.rs:729` (`pub trait Vmm`), `:877` (`pub trait Device`).

Three independent pieces of evidence that this is not QEMU in disguise:

1. **Two real implementations over different substrates already exist.**
   `crates/kayfabe-vmm-qemu/src/lib.rs:1945` (`impl Vmm for QemuVmm`) and
   `crates/kayfabe-vmm-kvm/src/lib.rs:1490` (`impl Vmm for KvmVmm`). The second is a
   `/dev/kvm`-direct harness with no hypervisor at all, and
   `crates/kayfabe-vmm-kvm/Cargo.toml:3` states the intent in the manifest: *"The FIRST real
   `Vmm`, and deliberately not a hypervisor's — a trait that has only ever been implemented
   against one hypervisor is a trait shaped by it."* That is the falsification test, run
   pre-emptively, and passed.
2. **The trait already carries a CH-conditional item, decided in CH's favour.**
   `crates/kayfabe-vmm/src/lib.rs:481-487` marks `IrqSpec::IntxLevel` backend-conditional because
   *"a cloud-hypervisor/rust-vmm adapter's legacy INTx path is a userspace IOAPIC that exists
   only on x86-64"*; `:863-868` argues `Device`'s `&self` (rather than `&mut self`) specifically
   so that a hypervisor dispatching MMIO through a `&self` bus callback —
   *"cloud-hypervisor's `BusDeviceSync::read(&self, …)"* — is not forced to throw the per-`Proc`
   sharding away. A second backend that had never been considered does not get a signature
   chosen for it.
3. **A CI gate mechanically keeps QEMU's API vocabulary out of 16 crates.**
   `.github/workflows/ci.yml:557-563` names the scope (`kayfabe-core`, `-completion`, `-fwd`,
   `-mmu`, `-gsp`, `-rmrpc`, `-arch`, `-abi`, `-device`, `-util`, `-trace`, `-isolate`, `-vmm`,
   `-rt`, `-shell`, `-chips`) and `:570` is the pattern:
   `\bBQL\b|bql_lock|qemu_mutex_lock_iothread|BQL_LOCK_GUARD|MemoryRegion|memory_region_|qemu_bh|QEMUBH|aio_bh|qdev_|VMStateDescription|\bbottom.?half|\bmain.?loop|iothread`.
   ★ It matches **API identifiers, never the vendor's name** (`:534-542`), which is why it needs
   no allowlist. ⚠ Its limits, stated so the green is not over-read: it is a **grep for QEMU's
   nouns**, so it cannot see a QEMU *assumption* expressed in neutral words, and its scope
   excludes the adapter crates by design — which is where seams 4 and 5 live.

### Seam 2 — a new `impl Vmm` → **ADDITIVE**, ~2 400 lines

`crates/kayfabe-vmm-qemu/src/lib.rs` is 2 364 lines. A CH equivalent is a new crate. This is the
cost the contract *predicts*, and it is the only line item on this list that is not a surprise.

### Seam 3 — ★★★ the memory plane is ALREADY DUPLICATED → **RE-DERIVE (a third time)**

`crates/kayfabe-vmm-kvm/src/lib.rs:647` — `pub(crate) struct Plane`
`crates/kayfabe-vmm-qemu/src/lib.rs:777` — `pub(crate) struct Plane`

Both carry the same fields (`page`, `shareable_ram`, `bars`, `view`, `installer`, `clock`,
`audit`) and the same `view()` / `installer()` methods with the same bodies, differing only in
`leaf::` vs `leafwitness::` and in the backend handle at the top. `l2_qemu_adapter.md:1085` says
so in the plan, in as many words: stage Q1 is *"Ported from `kayfabe_vmm_kvm::Plane`
shape-for-shape."*

★★ **This is the measurement that most directly contradicts the contract** (read 2026-08-09 at `crates/kayfabe-vmm-kvm/src/lib.rs:647` and `crates/kayfabe-vmm-qemu/src/lib.rs:777`)**.** The contract's cost
model is *"one adapter crate"*, and the second adapter's crate contained a re-derivation of the
first adapter's core data structure. A third backend pays it a third time. The duplication is not
an accident — it was planned and named — but *"one adapter crate"* prices it at zero and it is
not zero.

★ Note what the tree already did once, correctly, with a smaller instance of the same problem:
the leaf-lock witness was built in `kayfabe-vmm-kvm`, immediately recorded as being in the wrong
home, and moved to `kayfabe_util::leafwitness` when a second consumer appeared —
`crates/kayfabe-vmm-kvm/src/leaf.rs` is now a 24-line re-export whose module docs are the whole
finding (*"it belongs beside `lockwitness`… the first time a second adapter needs it"*). The
`Plane` is the same shape and has not had that treatment.

### Seam 4 — 1 682 lines of VMM-NEUTRAL code inside the vendor-named crate → **EXTRACT**

Counted with
`for f in crates/kayfabe-vmm-qemu/src/*.rs; do grep -cE 'QemuHost|MrHandle|SectionFacts|SectionDesc|BarPlacement|region_add|region_del|migrate_|ram_block_discard|kvm_enabled|listener' $f; done`:

| file | QEMU-specific identifier hits | lines |
|---|---:|---:|
| `crates/kayfabe-vmm-qemu/src/viewer_install.rs` | **0** | 1 030 |
| `crates/kayfabe-vmm-qemu/src/slots.rs` | **1** | 652 |
| `crates/kayfabe-vmm-qemu/src/lib.rs` | 43 | 2 364 |
| `crates/kayfabe-vmm-qemu/src/mock_host.rs` | 50 | 689 |
| `crates/kayfabe-vmm-qemu/src/host.rs` | 30 | 328 |
| `crates/kayfabe-vmm-qemu/src/classify.rs` | 13 | 152 |

`viewer_install.rs` (the GPGA viewer → memslot installer) and `slots.rs` (the three-tier memslot
plane) name **almost nothing** of QEMU's. `slots.rs:1-9` says so itself: *"installing a memslot is
a call to the **kernel**, so putting it on the trait whose subject is the hypervisor's C API would
make it untestable"* — the module was deliberately kept off `QemuHost`, and then placed in the
QEMU crate anyway.

⇒ A CH adapter wanting either must depend on `kayfabe-vmm-qemu` (and therefore on `QemuHost`), or
copy 1 682 lines. **Neither is "one adapter crate".** The fix is a crate rename plus a move — e.g.
a `kayfabe-vmm-kvmplane` holding `slots.rs` + `viewer_install.rs`, which both adapters depend on —
and it is cheap **now** and gets monotonically more expensive.

### Seam 5 — ★★★ THE COMPOSITION ROOT LIVES IN THE QEMU FFI CRATE → **EXTRACT (6 sites)**

This is the largest finding and the one a reader is least likely to guess.

`crates/kayfabe-qemu-raw/src/shim.rs` is 3 620 lines. Its four sections, by banner line:

| lines | section | subject |
|---:|---|---|
| 1–667 | the shim proper (`Status`, `classify`, `BarDesc`, `ShimConfig`, `SectionWire`, `Shim`) | **QEMU** |
| 668–2023 | *"The register plane (stage Q4) — the safe half"* (`:669`) — `MachineRam`, `KayfabeChipIdentity`, `chip_for`, `publication_row`, the `Kayfabe*` audit rows, `Regs` (`:2621`) | VMM-neutral |
| 2024–3431 | *"E2 — the join between a trapped BAR write and `kayfabe_rt::SharedDevice`"* (`:2025`) — `SharedObjectModel`, the object-model bridge | VMM-neutral |
| 3432–3620 | *"The isolate-plane selector"* (`:3433`) — `ISOLATE_PLANE_ENV`, `isolate_plane_from`, `isolate_factory` | VMM-neutral |

**2 953 of 3 620 lines (81.6 %) are VMM-neutral device composition**, and the manifest proves it:
`crates/kayfabe-qemu-raw/Cargo.toml` pulls in `kayfabe-core`, `kayfabe-device`, `kayfabe-rmrpc`,
`kayfabe-gsp`, `kayfabe-rt`, `kayfabe-chips`, `kayfabe-abi` and `kayfabe-isolate` — the **entire
stack** — into a crate whose stated subject is *"the entire foreign-function surface of the
hypervisor adapter"* (`crates/kayfabe-qemu-raw/src/lib.rs:3-5`).

★★ **Now the number that decides whether this is an extraction or a rewrite.** Over those 2 953
lines, how many name a QEMU-specific type?

```console
$ awk 'NR>=668' crates/kayfabe-qemu-raw/src/shim.rs \
  | grep -cE 'QemuMachine|QemuVmm|QemuHost|MrHandle|SectionWire|SectionDesc|BarPlacement|kayfabe_vmm_qemu'
6
```

**Six**, and all six are the single type `QemuVmm` — `shim.rs:729` (a doc line), `:734`
(`MachineRam { vmm: QemuVmm }`), `:740` (its constructor), `:2193`, `:2202`, `:2856`. Every one
would become `Arc<dyn Vmm>` or a generic parameter.

⇒ **Verdict: ADDITIVE, but only after an extraction, and nothing in the tree forces the
extraction.** The vocabulary gate's scope (`ci.yml:557-563`) is 16 crates and `kayfabe-qemu-raw`
is deliberately not one of them — correctly, since it must name QEMU's API — which means the
2 953 VMM-neutral lines that drifted in are in the one place no gate is looking. **This is the
"unhooked implementation" shape of task #121, one layer up: the composition root is a real,
working, general implementation that a second backend cannot reach.**

★ Cheapest durable countermeasure, since a grep cannot express *"this crate contains too much"*:
a **line-count ratchet** on `crates/kayfabe-qemu-raw/src/shim.rs`, in the style of the existing
per-crate unsafe ratchet at `ci.yml:1413` (`AUDITED="kayfabe-linux-raw:90 kayfabe-qemu-raw:48"`).
It cannot say what belongs there, but it makes the next 500 lines of composition a **decision**
instead of a commit — which is the same trick the unsafe ratchet already plays, and it is the
only enforcement shape that has worked on this axis.

### Seam 6 — `QemuHost` is genuinely QEMU's API → **RE-DERIVE, ~330 lines, and correctly so**

`crates/kayfabe-vmm-qemu/src/host.rs:175` — 14 methods, of which `migrate_add_blocker` (`:199`),
`ram_block_discard_disable` (`:231`), `register_listener` (`:238`), `kvm_enabled` (`:191`) and
`bar_is_unbacked_reservation` (`:259`) are QEMU concepts wearing QEMU's names.

⊘ **This is not a defect.** It is the *adapter's own* seam, one layer below `Vmm`, and `host.rs:11`
states the direction: *"The trait is defined by the CONSUMER, in the safe crate, so the FFI crate
has no say in the shape of the port."* A CH adapter writes a different trait with different
methods; that is what an adapter is. Counted here only so the ~330 lines are in the total.

### Seam 7 — `classify.rs` → **RE-DERIVE, 152 lines**

`crates/kayfabe-vmm-qemu/src/classify.rs` classifies `MemoryListener`-reported `SectionFacts`
into `RegionKind`. Its five-field test exists because *"`is_ram` alone is wrong in three
independent directions and the complete test is not expressible as one predicate at the pinned
version"* (`host.rs:98-101`). Those three directions are facts about QEMU's region types; CH's
equivalent must be re-derived from CH's. **The rule it enforces is portable and lives in the port
crate** (`kayfabe-vmm/src/lib.rs:223-266`, `GuestRamMap`); only the evidence is backend-specific.
That split is exactly right and is the pattern to copy.

### Seam 8 — the C QOM shim and the FFI table → **DELETE on CH**

`qemu/hw/misc/nvkvm/{nvkvm.c, nvkvm_compat.h, kayfabe_shim.h, meson.build}`,
`scripts/build_qom_shim.sh`, and the 18 `extern "C"` entry points in
`crates/kayfabe-qemu-raw/src/shim_unsafe.rs` (`:716`–`:1247`).

★ **Cloud Hypervisor is Rust.** A CH device is a `BusDeviceSync` implementor registered in-process
— no C, no QOM macro system, no hand-mirrored function-pointer table, no ABI/`sizeof` handshake,
and **no second `unsafe`-allow crate** (the price `l2_qemu_adapter.md` §2.2 / DECISION Q2 paid, and
whose cost it stated as *"the audit acceptance criterion moves from 'read one crate in one sitting'
to 'read two'"*). This seam is **cheaper on CH than on QEMU**, and it is ~1 260 lines of `unsafe`
that simply do not exist there.

### Seam 9 — the memslot allocator's collision discipline → ✅ **ADDITIVE, already designed for this**

`crates/kayfabe-vmm-qemu/src/slots.rs:206` (`SlotAllocator`). Slot numbers descend from the
kernel's ceiling rather than ascending from a hardcoded base *"on a convention enforced by
nothing"* (`crates/kayfabe-vmm-qemu/src/lib.rs:44-45`, contrasting the C). Installing our own
memslots into a machine another process created is exactly the CH situation too. Reusable as-is
— **once seam 4 makes it reachable.**

### Seam 10 — the foreign-lock class → ✅ **ADDITIVE; CH's instance is already written down**

`crates/kayfabe-vmm/src/lib.rs:39-44` states the class portably (*"no lock the VMM owns … may be
acquired beneath one of our locks"*) and names both instances: QEMU's whole-machine lock, and
*"cloud-hypervisor's … per-device mutex its bus takes across the entire MMIO callback."*
`l1_os_shell.md:3080` goes further and records a **prediction with its mechanism**:
*"CH — unconditional pass expected: MMIO dispatch is a synchronous `VmOps` call on the vCPU
thread with no VM-wide lock, and with the direct-`BusDeviceSync` registration (§6.3) not even a
per-device one. A CH failure here would mean the escape was not taken."*
⇒ [assumed] That row is a source reading of CH that this audit did not re-run. It is a
**falsifiable prediction with a named failure condition**, which is the right shape, but it is not
a measurement and must not be cited as one.

### Seam 11 — ⚠ **THE UNKNOWN: can CH express an unbacked reservation BAR?** → **possibly RE-DERIVE**

`crates/kayfabe-vmm-qemu/src/host.rs:242-259`. The entire safety argument for installing our own
memslots over a hypervisor's BAR is that the reservation BAR **never sets the region's RAM flag**,
so the accelerator's listener takes an unconditional early return for it and allocates no slot of
its own — cited to `qemu: system/memory.c:1568-1579` and `accel/kvm/kvm-all.c:1449-1457`. The trait
method exists precisely so *"the adapter refuses to place a window in a BAR that does not report
this, rather than trusting that whoever wrote the shim read §1.5."*

**Whether Cloud Hypervisor's device model can produce a BAR with that property is not known, and
nothing in this tree answers it.** If it cannot, CH's memory plane is not a port of ours — and
`l2_qemu_adapter.md:1055-1056` already describes what that outcome looks like, for the vfio-user
case: *"It is a different architecture, not a different adapter: the `Vmm` port survives, but the
device model, the trap path and the memory plane are all re-derived."*

⇒ ★ **This is the one experiment that would move the overall verdict**, and it is a source read,
not a build: find CH's PCI BAR registration path and determine whether a BAR can be registered
such that CH installs no KVM memslot for it. Until then, the totals in §3 carry an asterisk.

---

## 3. The totals

| | lines | verdict |
|---|---:|---|
| **Zero change** (port, `Device`, `GuestRamMap`, `DeferQueue`, the whole core) | 914 + the 16 gated crates | ✅ |
| **Reusable once extracted** (seam 4: `slots.rs`, `viewer_install.rs`) | **1 682** | **EXTRACT** |
| **Reusable once extracted** (seam 5: the composition root, 6 identifier sites) | **2 953** | **EXTRACT** |
| **Genuinely new CH-specific adapter** (seams 2 + 6 + 7) | ~2 850 | ADDITIVE |
| **Duplicated a third time** (seam 3: the memory `Plane`) | inside the ~2 850 above | ⊘ RE-DERIVE |
| **Deleted on CH** (seam 8: the C shim + FFI table + second unsafe crate) | ~1 260 unsafe + 4 C files | ✅ DELETE |
| **Unknown** (seam 11) | — | ⚠ could re-derive the memory plane |

**"Additive" or "a rewrite"? — ADDITIVE, with two named debts and one open question.** The
abstraction is real, it was falsification-tested by its own authors, and it is CI-gated. What is
*not* true is the price: **4 635 lines of VMM-neutral code sit in vendor-named crates**, coupled to
QEMU on **6 identifier sites**, in the one region of the tree the portability gate deliberately
does not scan.

⚠ **And the direction of travel is the finding.** Every one of those 4 635 lines was written after
the contract, by people who had read it, in crates the gate excludes for good reasons. That is not
carelessness — it is what the contract's own text predicts of itself:

> *"it is the one property that decays silently — the first backend's vocabulary drifts inward one
> identifier at a time, and by the time a second adapter is attempted the port describes one
> hypervisor's API rather than a hypervisor's capabilities."*
> — `crates/kayfabe-vmm/src/lib.rs:20-23`

The prediction was right about the mechanism and wrong about the **location**. The port did not
drift; the port is clean and gated. What drifted was everything the gate does not cover, and it
drifted by *accumulation of neutral code in a vendor-named crate* rather than by leakage of vendor
vocabulary into a neutral one. **A gate that watches for the vendor's nouns cannot see that.**

★ The cheapest thing that would have caught it, and the recommendation: **the line-count ratchet of
seam 5**, plus moving `slots.rs` and `viewer_install.rs` out of `kayfabe-vmm-qemu` now, while the
move is a `git mv` and a `Cargo.toml` line.

---

## 4. ⊘ What this audit did NOT do

- ⊘ **No Cloud Hypervisor source was read**, in this audit or (as far as its citations show) in
  the tree. Seams 10 and 11 rest on readings recorded by others, and seam 11 is unanswered. Any
  sentence here about what CH *does* is `[assumed]`; every sentence about what **our** tree does is
  a count over a named file.
- ⊘ **Nothing was built and nothing was booted.** All counts are static.
- ⊘ **No adapter was written**, per the audit's own scope. The 6 identifier sites of seam 5 are a
  count, not a patch.
- ⊘ It does not claim the extraction is *safe* — only that it is **small**. Whether
  `MachineRam`'s `QemuVmm` can become `Arc<dyn Vmm>` without a lifetime or object-safety problem
  is a compile away, and this audit did not compile it.

---

## 5. Consequences to carry forward

1. **Stop quoting *"one adapter crate, zero trait changes"* as a cost model.** The trait half is
   true and gated. The crate half was already refuted by our own second adapter (seam 3). Quote
   `ARCHITECTURE.md:31` with that caveat, or amend it.
2. **Seam 4 and seam 5 are the whole debt**, they total 4 635 lines, and both are cheap today.
3. **Seam 11 is the only thing that could change the verdict.** It is a source read of Cloud
   Hypervisor's BAR registration, not a build.
4. **The vocabulary gate is sound and must not be widened to the adapter crates** — it works
   *because* it excludes them. The gap it leaves is a different gate's job (a ratchet), not a
   bigger grep.

---

## 6. Appendix — the QEMU-backport contradiction, site by site

`vmm_integration_and_support_matrix.md` §6 rules the substance: **a version floor is fine, a
maintained fork is what to avoid.** This appendix records the sites edited on 2026-08-09, because
the audit that found the contradiction had itself gone stale and sent readers to the wrong lines.

| # | site | before | after |
|---|---|---|---|
| 1 | `l1_os_shell.md:78-81` | **CONTRADICTION-OPEN** — *"decisions #35 and #48 all still treat the backport as the remedy"* | **RESOLVED**, struck, cross-linked; and corrected — #35/#48 had already been fixed on 2026-07-28 |
| 2 | `l1_os_shell.md:1532-1537` | ★ **REMEDY**, present tense — *"The remedy — the 10.2.0 backport"* | banner: **the remedy is the FLOOR**; the paragraph is retained as the historical measurement |
| 3 | `../reference/qemu_bql_spike.md:11-15` | **two caveats, neither of them the cancellation** | **three caveats**; caveat 3 states the backport is cancelled and voids §5 limit 4's soak risk |
| 4 | `l2_qemu_adapter.md:205` (law L1) | CANCELLED **+ three wrong line citations** | citations corrected in place; cross-linked to the ruling |
| 5 | `l2_qemu_adapter.md:1208-1211` (§12a item 11) | *"Five sites still describe a carried backport"* — wrong count, wrong lines, two wrong names | **STRUCK** with the re-read; the live sites named correctly |
| 6 | `open_questions_for_the_owner.md:855-856` (Q23 item 1) | **CONTRADICTION-OPEN** | **DECIDED**, cross-linked |
| 7 | `open_questions_for_the_owner.md:48-50` (doc-drift list) | *"the QEMU-backport prose survives a CANCELLED ruling"* | struck as fixed, with the two real sites named |

★★ **Why it survived four correction passes, which is the transferable part.** Decisions #35 and
#48 were corrected on 2026-07-28. The **reports** of the contradiction were not, and two of them
(`l2_qemu_adapter.md:205`, `:1208`) cited `l1_os_shell.md:1299`, `:2879` and `:3211` — which are,
respectively, a bare `>`, a sentence about the retire/reap discipline, and a sentence about
reconciling §7. **None is about the backport.** Anybody who checked the report found three
innocent sentences and reasonably concluded the report was noise.

Meanwhile the one file that genuinely still read as a live plan to carry a patch — `qemu_bql_spike.md`
— was named by **no** report, and it declares at `:19-21` that *"where this file and a design doc
disagree, this file wins and the design doc gets amended."*

⇒ [[a-wrong-citation-is-more-durable-than-none]], with a rider: **a contradiction report is itself a
citation, and it decays exactly like one.** A stale report is worse than no report, because it
converts a real contradiction into something that looks already-handled. And a **reference** that
out-ranks the design docs and lags them does not merely go stale — it silently **re-authorises**
the position the design docs retracted.
