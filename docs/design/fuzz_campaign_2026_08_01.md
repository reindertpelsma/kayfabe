# The VMM-escape fuzz campaign — what it found, and what it says about the boundary

> Campaign run 2026-07-31 → 2026-08-01 on the 38-core build box: **7 libFuzzer targets**,
> `-fork=4` each, `-max_total_time=2700`, `-rss_limit_mb=4096`, `-max_len=65536`.
> Corpora reached 792 K … 6.2 M per target. Branch `fuzz-hardening`.

## 1. What was fuzzed, and why these seven

The owner's boundary statement scopes this exactly:

> *"we are just a device implementation in vmm, like extension, not the vmm itself. our
> boundary stops at vmm escape, which is already very bad, the rest is upstream
> responsibility"*

So the surface worth fuzzing is **everything the guest can put bytes into that we then
interpret**. The seven targets:

| target | the untrusted input |
|---|---|
| `abi_decode` | guest-supplied ioctl/RPC parameter blocks |
| `gsp_msgq` | the GSP message-queue ring header + element stream |
| `gsp_region` | a declared region: page list × page size |
| `parse_pushbuffer` | guest pushbuffer method stream |
| `pt_walker` | guest page-table entries |
| `rpc_bridge` | the RPC framing layer |
| `gpga_index` | the viewer index — GPGA regions and view offsets |

## 2. The findings: four, and all one shape

★★★ **Every defect found was an unchecked arithmetic domain**, not a logic error. That is
worth stating plainly, because it says the *decision* logic held up under 45 minutes of
coverage-guided pressure and the *arithmetic* did not.

| # | site | the sum | consequence unchecked |
|---|---|---|---|
| 1 | `kayfabe-mmu/src/gpga.rs` | `view_off + region.len` | wrapped → a viewer gets a slice whose `view_off` points somewhere it never mapped |
| 2 | `kayfabe-gsp/src/ram.rs` | `pages × page_size` | wrapped → the value `RegionMap::runs` tests every access against **is a bound computed from an overflow** |
| 3 | `kayfabe-gsp/src/ring.rs` | `read_ptr + n` | wrapped → a wrong free-count **desynchronises the ring silently** |
| 4 | `kayfabe-gsp/src/element.rs` | `count × element_size` | wrapped → a layout that does not describe the elements |

In a debug build each was a **panic** under `overflow-checks`; in release each **wrapped**.
⊘ The wrap is the dangerous half: a panic is a loud VMM abort (bad, but bounded and
diagnosable), while a wrapped bound is a **silently wrong address or length** that the rest
of the system then trusts.

## 3. ★★★ Reachability — stated honestly, because three of the four are NOT reachable

This is the part an audit write-up usually gets wrong in the flattering direction. Of the
four:

- **#2 `gsp_region`** — ⚠ **not reachable from a guest today.** The only production caller
  passes `MsgqAbi::region_page_size` (`RM_PAGE_SIZE`, a driver constant) with a 4096-entry
  cap, so the product is 16 MiB. Fixed anyway because `load`'s *declared* domain is any
  non-zero power of two.
- **#3 `gsp_msgq`** — ⚠ **not reachable through `MsgqGeometry::bind`.** `rx_link_check`
  requires `msgCount == (size - entryOff) / msgSize` with `msgSize >= 16`, so a bound ring's
  count is at most `u32::MAX / 16`. Fixed anyway because the precondition that saves it lives
  in a **different function** and is not stated on this one.
- **#4 `element`** — ⚠ **not reachable through a bound geometry**, same guard.
- **#1 `gpga_index`** — reachability **not established**; see §4, which is where it led.

★★ **The argument for fixing an unreachable one** is not "defence in depth" hand-waving. It
is that a `pub fn` whose declared domain is `MsgCount` must be **total over `MsgCount`** — a
type whose stated contract is wider than its arithmetic is a bug waiting for its second
caller. The guard that saves it is real but lives elsewhere, and nothing links the two.

## 4. ★★★ The finding the fix itself exposed — a boundary check that stopped at a crate line

The `gpga_index` fix (#1) is placed where the offset **enters** the index, with an explicit
argument: the three later consumers *"have no standing to re-litigate it"* because by the time
they run the offset is already committed. Its rustdoc enumerates those three —
`ViewUpdate::Shows`, `ViewerIndex::viewers_of`, `ViewerIndex::view_contents`.

⚠ **All three are inside `kayfabe-mmu`.** There is a **fourth** consumer, in another crate:

```rust
// crates/kayfabe-vmm-qemu/src/viewer_install.rs — place_content
vmm.map_guest(self.gpa + view_off, len, backing, Prot::ReadWrite)?;
```

This adds **`self.gpa`**, the window's base GPA — *not* a within-region delta, and a value the
index has never seen. So the index's new invariant (`view_off + region.len` fits) does **not**
bound it: two different sums, and bounding the first constrains the second only if you already
know `self.gpa` is small, which nothing states.

Unchecked, this hands `map_guest` a GPA the view never described — **guest memory installed at
an address nobody chose**, which is squarely inside the declared blast radius. Now refused as
`InstallRefusal::MappingGpaOverflows`, at the per-placement site *and* once over the whole
covered set before the first `map_guest`.

★★★ **The transferable lesson:** *"checked once, at the boundary"* is only as strong as the
**enumeration of consumers** behind it — and an enumeration written inside one crate will
naturally stop at that crate's edge. This is [[gates_quantified_over_a_list]] wearing different
clothes: not a list that was shortened, but a universe that was drawn smaller than the fact it
was supposed to cover. ⇒ Of every such argument, ask: **what is the set of consumers, and does
it cross a crate line?**

## 5. What the campaign does NOT establish

- ⊘ **No memory-safety finding, and that is nearly uninformative.** The workspace forbids
  `unsafe` outside `*_unsafe.rs`, so an out-of-bounds access is a panic by construction. These
  targets exercise safe code; they can find wrong *values*, not corruption.
- ⊘ **No statement about the completion plane**, which has no oracle at all
  (`c_rust_trace_differential.md`).
- ⊘ **Coverage is not measured here.** 45 minutes per target with a warm corpus is a
  smoke-level campaign, not an exhaustive one. Absence of further findings is **not** evidence
  of absence.
- ⊘ **The 13 crash artifacts in `fuzz/artifacts/` no longer reproduce** against the fixed
  build (all exit 0). They are kept as regression inputs. ⚠ Their non-reproduction was checked
  against a binary built *after* the fixes — it is consistent with the fixes working, and it
  would look identical if a harness change had merely stopped reaching the path. The bite tests,
  not the artifacts, are what hold these closed.

## 6. Standing recommendation

Re-run the campaign whenever a decoder changes, and **treat a new arithmetic-domain finding as
a design question, not a patch**: all four here were a function whose declared domain was wider
than the arithmetic inside it, which is a *type* problem that a `checked_add` only papers over
at one site.
