# The architecture, v2 — lazy everywhere, strict at recycle

**STATUS: LIVE, written 2026-09-20 (w815). Supersedes `ARCHITECTURE_from_scratch.md` and
`l1_architecture_summary.md`, both of which describe the two-worlds framebuffer and the address
table this document deletes.**

⊘ This is a DESIGN document, not a description of the tree as it stands. Where the code
disagrees with this document, the code is what is scheduled to change — each such place is
marked **[ROT]** with what it costs today. Nothing here is a claim that it already works.

---

## 0. The one-paragraph version

A stock NVIDIA driver runs in a KVM guest against an emulated GPU and a faked GSP. Guest video
memory **is** one reserved device-local RM object on the host, and a guest framebuffer offset
**is** a file offset into it — the identity is the whole memory model. The guest's own page
tables are walked on the GPU (205.7 µs) and the diff is applied as `mmap`/`munmap` against a
host address space, so **the host page tables are the address model** and nothing mirrors them.
A doorbell is never delayed for that refresh. The only ordering that is load-bearing anywhere
is that a physical page may not reach a second guest client until its old translations are gone
and it has been **scrubbed on hardware**.

---

## 1. What we are allowed to be wrong about, and what we are not

★★★★★ **THE CENTRAL RULE OF v2, and it is the guest's own rule.** ogkm — the honest guest
driver — already accepts that until an invalidate has *completed*, addresses are **undefined**.
It does not accept that they may be **unsafe**. Concretely, a translation may resolve to the
old page or the new page; it may never resolve to a page neither mapping ever referenced.

| | permitted | forbidden |
|---|---|---|
| a stale translation (old page) | ✔ the guest tolerates it by construction | |
| a fresh translation (new page) | ✔ | |
| a fault | ✔ — it is not a third thing | |
| **a page neither mapping referenced** | | ⊘⊘⊘ **this is the only memory-safety failure** |

⇒ Everything expensive in v1 bought *freshness*, which the guest never asked for. v2 spends
nothing on freshness.

★ And the third row is not something we must build machinery to prevent: the guest already
prevents it, by scrubbing a page before handing it on. We reach it only by **telling the guest
a scrub happened when it did not** (§5). The fix is to stop lying, not to add an invariant.

⊘ **`[measured w814]` We are currently violating it.** `--cross-client-leak`: client A writes
through its own VA and client B's object changes (`R5b B saw A's 2nd`). The same arm on bare
metal reports `R5b ISOLATED … leak A<-B=false leak B<-A=false`. That is the third row, and it
is ours.

---

## 2. Memory: one store, identity offsets

- Guest vidmem is **one** reserved device-local RM object, allocated once at realize.
- Guest FB offset **is** the offset into that object. There is no allocator, no per-page
  fabrication, no second world.
- Guest sysmem is the hypervisor's `memfd`, reached by a **layout** (GPA range → fd + offset),
  which is O(memslots) and given to us, not observed.

**What this deletes outright:**

- ⊘ **Joins.** `join_one_fb_leaf`, `FbJoinArm`, `join_release` and its supersede/alias arms,
  the `ALREADY JOINED` / `CANDIDATE` accounting. A join exists to give an *emulated framebuffer
  page* a host object; under one store every page already has one, by construction.
  **[ROT]** `[measured w812]` three gates of that machinery were what kept the guest's CE work
  off the GPU: `join_one_fb_leaf` returned `None` for every leaf, operands stayed
  `/FakeFramebuffer`, every copy graded `CeExecutor::Ours`.
- ⊘ **The two-worlds framebuffer** (`twoworlds.rs`, the arena arm, demand-fill mirrors).
- ⊘ **`FbStoreArm::Arena`** — the surviving control arm of a switch that has been made.

---

## 3. Addresses: the host page tables ARE the model

★ **There is no address table.** The PTX walk kernel walks the guest's page tables on the GPU
and produces a **diff**. The diff is a set of `mmap`/`munmap` calls. After applying it, the
answer to *"what does this VA resolve to"* lives in exactly one place: the host address space.

Every field a table would have held is already present at the moment you act:

| a table would store | where v2 gets it |
|---|---|
| target of the mapping | identity — `mmap(store, offset = fb_offset)`; no lookup |
| sysmem target | the static layout; O(memslots), not O(pages) |
| permissions, aperture, size | the PTE just walked |
| "it went away" | `munmap` |

**What this deletes outright:**

- ⊘ **VA→GPA translation on the submission path.** Channels are born in the birth client with
  the guest's own VA space duped (route K), so the guest's VA *is* the host's VA. A translation
  step would be an identity function that can fail.
- ⊘ **The operand gate.** We currently pre-validate CE operands — *"the copy named an operand
  page this channel's VA space binds nowhere"* — because we do not trust the mapping to exist.
  In v2 the GPU resolves operands itself and **faults** if we were wrong, which is what real
  hardware does. ⚠ The C artifact went green *without servicing a single GPU fault*, because a
  wholesale sweep made faults impossible as a class rather than by checking each one.
- ⊘ **The publication machinery**: publication epochs, `VasPublishArm`, the w318 dirty gate,
  `pt_sweep_skip`, `candidates/published/refused` census rows, `UnknownPdb`, `CrossesEnd`.
  **[ROT]** these are *reconciliation* failures — two copies of one fact disagreeing — and a
  design with nothing to reconcile cannot have them. Most of 2026-09-19/20 was spent inside
  them.
- ⊘ **`Pdb` as an identity.** **[ROT]** `Pdb` currently means both *"which address space"* and
  *"the physical address of its page directory"*, and `vas.pdb.unwrap_or(Pdb(0))` at eight
  call sites makes `Pdb(0)` mean both *"declared at framebuffer offset 0"* and *"no declaration
  yet"*. `[measured w812]` that single collision produced **four** consecutive wrong diagnoses,
  each a true statement about the wrong population. In v2 an address space is named by its
  guest resource key; a root is `Option<Pdb>` and is needed **only** to seed a walk.

---

## 4. The three channel kinds

| kind | ring | who executes | we inspect |
|---|---|---|---|
| **Passthrough** | the guest's own | the GPU | never — parsing, not placement, is what is forbidden |
| **Translated** | ours; the guest's is a fake in GPGA | the GPU, from a rewritten pushbuffer | every entry |
| **Emulated** | ours | **us** — it is a function we implement | n/a: there is no GPU counterpart |

★ `Emulated` is for channels that have no hardware meaning for us to forward — the UVM kernel
channel, things we must stub. ⊘ It is **not** a fallback for "we could not translate this".
A translated entry we cannot rewrite is refused by name, or forwarded to a channel that
deliberately faults so the guest's own error reporting works untouched.

**[ROT]** `CeExecutor::Ours` — the CPU byte-copy — is not one of these three kinds. Under §46
real kernel-channel work is never executed on the CPU. When that lands,
`CeExecutor::Ours`, `CE-LOCAL-REFRESH` (132 per run), `DEFERRED-LOCAL`, the CE-split/`commit_ce`
path and `KAYFABE_CE_EXECUTOR=local` all delete together.

---

## 5. The scrub: the guest already does it — we must stop lying about it

★★★★★ **We own no scrub policy.** The guest's own RM scrubs a page before it reaches another
client, and it orders that correctly by itself. There is no invariant for us to enforce, no
"tear down translations then scrub then re-issue" sequence for us to implement, and no lifetime
question for us to answer.

⊘ **Our only failure is that we forge the completion.** `[measured w813]` the kernel tokens
carrying that work are `tok=0x00010001 emulated=74 forwarded=0` and
`tok=0x00010004 emulated=66 forwarded=0` — 140 doorbells rung and **not one sent to the GPU**.
We report the scrub complete; no bytes were cleared. The guest then reuses the page, correctly
by its own rules, and it still holds the previous client's data.

⇒ **That is the cross-client leak.** `R5b B saw A's 2nd` is not a staleness bug and not an
ordering bug — it is a page handed over unscrubbed because we said it had been scrubbed.

**The fix is one thing: let the scrub's work reach the hardware.**
`FwdFault::SystemDataPlane` refuses the system proc a data plane, implementing
`l1_concurrency.md` §12.26 — *"the CeUtils scrub … is **forged** … never forwarded, so the
system proc never mints host memory."* §46 names that exact example and reverses it:
*"Ceutils scrub is not a no-op, it genuinely needs to clear memory. Same for kernel ce."*
⊘ §12.26's stated reason is obsolete under one store anyway: the store is minted **once at
realize**, so a pin mints nothing.

★ **The gate already exists.** Those two tokens must read `forwarded > 0`. Nothing new needs
building to know whether this is fixed.

⊘ And the unmap half needs no special mechanism either. **[ROT]**
`the_store_slice_unmap_was_never_wired` records that nothing ever removes a store slice — but
in v2 removal is simply the `munmap` half of the walk diff (§3). It stops being its own
mechanism with its own bugs.

⚠ Out of scope, and must not be conflated with it: `ce_copy(Ours)` refusing inside the isolate
is a **different** refusal — the sandbox declining to hold guest memory — and stays.

## 6. The hot path, end to end

1. Guest writes a doorbell (`NV_VIRTUAL_FUNCTION_DOORBELL`, BAR0 `0x00BB0090`).
2. The trap does one allowlisted thing — update a queue, wake a worker, one dword for a
   passthrough doorbell, or a synchronous read-register write (§41). **No blocking work ever
   runs on a vCPU.**
3. A worker picks it up. **It does not wait for a page-table refresh** (§1).
4. Passthrough: the GPU reads the guest's ring. Translated: we read the ring, rewrite operands,
   submit. Emulated: we perform the function.
5. Completion is the guest's own semaphore, at the guest's own address. We do not forge it and
   we do not forward the guest's spin.

★ Refresh runs **beside** this, not inside it: walk, diff, apply, repeat. A doorbell that races
a refresh gets old-or-new, which §1 permits.

---

## 7. What remains unresolved — named, not designed away

1. **Doorbell → channel.** A token must still reach a channel object. That is a small
   fixed-size map, not an address table, but it has an owner and a lifetime and this document
   does not specify them.
2. **The PTX diff's previous state.** A diff needs something to diff *against*. That is one
   shadow of the last walk — O(mapped pages) — and it is the one structure v2 keeps. ⚠ It must
   never be consulted to *answer* a translation; the moment anything reads it on the submission
   path it has become the address table again.
3. **Instance-block / PDB discovery.** `MAP_MEMORY_DMA` is a HAL stub on GSP-client parts, so a
   client-allocated VA space is never declared to us — `[measured w811]` zero
   `SET_PAGE_DIRECTORY` events in a whole boot. The walk needs a root. Unsolved.
4. **Syscall cost of the diff.** N changed pages ⇒ N mappings unless runs are coalesced.
   ⊘ **Unmeasured.** The walk is 205.7 µs and page tables change far less often than doorbells
   ring, so the diff should be small and bursty — but that is a prediction, not a measurement.
5. **Multi-GPU and non-GA10x parts.** The chip table holds exactly one profile (GA106);
   `ad10x`, `gh100`, `gb20x` exist in `kayfabe-chips` and are not in it.

---

## 8. What v2 keeps, deliberately

⊘ The instruments, and they are the reason this rewrite is safe to attempt at all: the
doorbell ledger (`forwarded=` per token), the censuses, refusals **by name**, the bare-metal
baseline, the hardware gate, the 30-arm suite. `[measured 2026-09-20]` the bare-metal baseline
is what turned nine guest failures from opinion into adjudication.

⚠ Also kept: every **measured** fact in a comment. The mechanisms in this tree are the
liability; the measurements are the asset, and a rewrite that discards them pays for them twice.
