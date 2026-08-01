# The guest→VMM decoder fuzz campaign — what it found, and what it says about the boundary

> Campaign run 2026-07-31 → 2026-08-01 on the 38-core build box. **8 targets**, `-fork=4`
> each, 2700 s wall each, all 8 concurrent (32 workers on 38 cores, so exec/s is *depressed*
> by contention — the counts below understate an uncontended run).
> Landed as `72fd20f`; this document is the write-up plus one follow-up finding (§4).

## 1. What was fuzzed, and how the target list was derived

The owner's boundary statement scopes this exactly:

> *"we are just a device implementation in vmm, like extension, not the vmm itself. our
> boundary stops at vmm escape, which is already very bad, the rest is upstream
> responsibility"*

⇒ the surface worth fuzzing is **everything the guest puts bytes into that we then interpret**.
The list was **derived** — every `pub fn` taking `&[u8]`, every guest-derived scalar reaching
arithmetic or an index — then cross-checked by an independent sweep, rather than guessed.

| target | execs | features | crashes |
|---|---|---|---|
| `abi_decode` | 113 625 548 | 888 | 0 |
| `isolate_proto` | 90 105 819 | 2 478 | 0 |
| `gsp_region` | 28 664 343 | 1 218 | 0 |
| `parse_pushbuffer` | 28 401 | 4 029 | 0 |
| `gsp_msgq` | 25 206 550 | 561 | 0 |
| `gpga_index` | 23 523 775 | 10 279 | 0 |
| `rpc_bridge` | 15 815 044 | 6 686 | 0 |
| `pt_walker` | 10 388 491 | 1 464 | 0 |

≈ **307 M executions**, zero surviving crashes at the end of round 3.

★★ **Two presumed targets do not exist, and saying so is part of the result.** There is **no
decompressor** anywhere in the tree — `regaccessmap.rs` only *frames* gzip (validates a magic
and a size on a `&'static [u8]` chip-row constant, and the served row is `NOT_PUBLISHED`,
empty). We hand the guest's RM a blob that **RM** inflates; we inflate nothing, and no
`flate2`/`miniz`/`zlib` appears in `Cargo.lock`. ⇒ compression bombs have no attack surface
here. Likewise the five "served controls" are **encoders** taking typed Rust arguments —
output-side correctness, not escape surface.

## 2. The findings: four real defects, and all one shape

★★★ **Every real defect was a `pub fn` whose declared domain is wider than its arithmetic's
safe domain, saved only by a precondition enforced in a _different function_.** Not one logic
error. That is the headline: the decision logic held under 307 M executions; the arithmetic
domains did not.

| # | site | the defect | severity |
|---|---|---|---|
| 1 | `gsp/src/ring.rs:321`/`:288` | `read_ptr + msgCount` overflows `u32`. `MsgCount` admits any non-zero `u32`; only `rx_link_check`'s `msgSize >= 16` caps it at `u32::MAX/16` | **Low** — fixed by widening to `u64` |
| 2 | `gsp/src/ram.rs:180` | `pages × pageSize` overflows — **and that product _is_ the bound `runs()` tests every guest access against** | **Medium** — a corrupted bound; OOB still stopped downstream by a `.get()`. Refused at construction |
| 3 | `mmu/src/gpga.rs:930` | `view_off + delta` unchecked at **three** sites (`:727`, `:930`, `:966`) — a wrapped window offset is a **mis-addressed mapping** | **Medium** — fixed once, where the offset enters |
| 4 | `gsp/src/element.rs:488` | `element_size == 0` builds a **zero-byte run and then indexes it**; the GSP-S1 gate is `0 > 0`, which is false | **Low** — fixed |

⊘ In a debug build each is a **panic** under `overflow-checks` (a loud VMM abort — bad, but
bounded and diagnosable). In a release build each **wraps**, which is worse: *a wrapped bound
is not a bound*, and everything downstream then trusts it.

★★ **None is guest-reachable today, and each was checked rather than asserted.** #1 and #4 are
capped by `rx_link_check`; #2's only production caller passes `RM_PAGE_SIZE` (a driver
constant) with a 4096 cap, so the product is 16 MiB. They were fixed anyway on **totality**
grounds: a type whose stated contract is wider than its arithmetic is a bug waiting for its
second caller, and the guard that saves it lives somewhere the signature never mentions.

### Two results that were *not* defects — recorded so they are not re-found

- **`malloc(4279107650)` OOM.** `encode_message` sizes from `element_size_max`, an ABI-table
  constant; the *harness* handed it 4 GiB. Harness bounded, reasoning recorded in the target.
- ★★★ **`gpga_index`'s own assertion was wrong — it demanded the wrapping behaviour `spans`
  explicitly refuses, i.e. _the test asserted the vulnerability_.** Caught in 22 execs.
  Coverage then went **306 → 1819**, so the bad assertion had been *blocking exploration* the
  whole time. This is `suspect_the_instrument_first` in its purest form: the instrument was
  both wrong and suppressing the evidence that would have shown it.

## 3. ★★★ The replay phase replayed nothing — and passed

`fuzz/.gitignore` ignored `corpus` and `artifacts`. On a fresh clone neither directory existed,
so `fuzz-corpus-replay` — whose own text says it *"replays the 114 committed corpus files AND
the committed crash artifact"* — replayed **zero files and exited green**.

⇒ The requirement that this be *"a permanent regression test rather than a story"* was
**structurally unmeetable**, and nothing was red. Fixed: un-ignored, **1408 files** committed
and actually replayed. Both fuzz phases are now quantified over `cargo fuzz list` rather than
naming `parse_pushbuffer` — which would have run **one of eight** and still reported green —
and `replay_fuzz_all` counts what it fed and goes **red** on a target with zero committed
inputs.

★★ Same family as [[gates_quantified_over_a_list]], twice over: a universe (`git ls-files`
minus a `.gitignore`) that excluded the very thing under test, and a gate quantified over a
one-element list.

## 4. ★★★ The follow-up finding: a boundary check that stopped at a crate line

Finding #3's fix is placed where the offset **enters** the index, with an explicit argument:
the later consumers *"have no standing to re-litigate it"*. Its rustdoc enumerates them —
`ViewUpdate::Shows`, `ViewerIndex::viewers_of`, `ViewerIndex::view_contents`.

⚠ **All three are inside `kayfabe-mmu`.** There is a **fourth**, in another crate:

```rust
// crates/kayfabe-vmm-qemu/src/viewer_install.rs — place_content
vmm.map_guest(self.gpa + view_off, len, backing, Prot::ReadWrite)?;
```

This adds **`self.gpa`**, the window's base GPA — *not* a within-region delta, and a value the
index has never seen. The index's invariant (`view_off + region.len` fits) does **not** bound
it: two different sums, and bounding the first constrains the second only if you already know
`self.gpa` is small, which nothing states. Unchecked it hands `map_guest` a GPA the view never
described — **guest memory installed at an address nobody chose**, squarely inside the declared
blast radius.

Now refused as `InstallRefusal::MappingGpaOverflows`, at the per-placement site *and* once over
the whole covered set before the first `map_guest`.

★ **Bite-verified 2026-08-01 at `be04de9`** (local dev box, `cargo test -p kayfabe-vmm-qemu
--test viewer_install`): replacing the `checked_add` with `+` turns
`a_mapping_gpa_that_would_not_fit_a_u64_is_refused_by_name` red with *"attempt to add with
overflow"* at `viewer_install.rs:882`, and it passes with the check restored.

⊘ Reachability **not established**, and not claimed — it rests on the same totality argument as
§2's three unreachable findings.

★★★ **The transferable lesson:** *"checked once, at the boundary"* is only as strong as the
**enumeration of consumers** behind it, and an enumeration written inside one crate will
naturally stop at that crate's edge. Of every such argument ask: **what is the set of
consumers, and does it cross a crate line?**

## 5. What the campaign does NOT establish — the gaps, named as gaps

- ⊘ **`parse_pushbuffer` managed 28 401 execs in 45 min (~5/s)** because it rebuilds a whole
  `Gpu` + `Scenario` per input. At that rate it is **sampled, not fuzzed**. Needs a reusable
  fixture. Pre-existing, not introduced here.
- ⊘ **`abi_decode` saturated at 888 features in ~40 s**, and 113 M further execs found nothing
  new. Either the decoders genuinely have little input-dependent branching (plausible — mostly
  a length check plus `from_le_bytes`), or the `arbitrary` shape is not reaching the `Ok`
  paths. **These were not distinguished**, so treat that green as *weaker than its exec count
  suggests*.
- ⊘ **Not fuzzed at all:** `scm_unsafe.rs` fd receive (needs a real socketpair with crafted
  `SCM_RIGHTS`, unreachable from `&[u8]`); `gh100.rs`/`ad10x.rs` MMIO register dispatch,
  whose EMEM window indexes on a guest offset; `kayfabe-crec/src/format.rs::parse`.
- ⊘ **`gsp_msgq` cannot reach `element_size_max > 65535`** by construction (harness bound,
  documented in the target). Nothing else covers it.
- ⊘ **No memory-safety finding, and that is nearly uninformative.** `unsafe` is forbidden
  outside `*_unsafe.rs`, so OOB is a panic by construction. These targets find wrong *values*,
  not corruption.
- ⊘ **Absence of further findings is not evidence of absence.**

## 6. Standing recommendation

Re-run whenever a decoder changes, and **treat a new arithmetic-domain finding as a design
question, not a patch**: all four here were a function whose declared domain was wider than the
arithmetic inside it, which a `checked_add` only papers over at one site.
