# Dirty tracking for one store, without userfaultfd

**STATUS: LIVE (2026-09-14, w720).** Design options and their arithmetic. ⊘ **Nothing here is
built**, and the measurement that arbitrates between them has not been taken.

## Why this file exists

The **single-store** simplification — everything in one reserved RM object, no fake fb, no two
worlds — is wanted (owner, 2026-09-14): *"no two worlds, no ram, everything vidmem, CE fast
enough, one RM object, one MMIO mmap, a lot of code deleted."* ⊘ Its one real cost is **dirty
tracking**: *"without dirty tracking we end up reading tens of megabytes, every single refresh"*
— 1178 refreshes a boot, so tens of GB of link traffic stolen from the workload.

⊘⊘ **`userfaultfd` is ruled out** (owner; `uvm_hmm.c:577` *"UVM doesn't support userfaultfd"*;
`nvkvm-pv/docs/internal/gpa-window-narrow-maxphyaddr.md:213`). Do not re-propose it.

## The options, and what each actually saves

| # | idea | saves the READ? | saves the DECODE? | new machinery |
|---|---|---|---|---|
| 1 | the invalidate names the edited range | **yes** | yes | small |
| 2 | a spare bit in the entry, cleared when RM rewrites it | ⊘ **no** | yes | small, + format risk |
| 2′ | **per-page hash, compare against last refresh** | ⊘ no | yes | trivial, **no format risk** |
| 3 | **GPU kernel diffs the tables in vidmem** | **yes** (link) | yes | ★ large |
| 4 | batch the CE copies into one push | n/a | n/a | small — **a precondition, not an option** |

### ⊘ Idea 2's structural flaw, and why 2′ dominates it

To see the marker was cleared you must **read the entry**. Scanning every entry to check its bit
pays the transfer you were avoiding. ⇒ It saves the *downstream* work, not the copy.

★ That may still be the larger half: reading 24 MB by CE at 10 GB/s is ~2.4 ms, while decoding
~960k entries at even 10 ns each is ~9.6 ms. **The decode likely dominates the copy.**

⊘ But **hashing (2′) gives the same answer with none of the risk**: no assumption about which bits
the MMU ignores, no risk that RM reads back an entry and trips on a stray bit, and no dependence
on hardware behaviour the open source cannot describe. ~0.2 ms for 1872 pages. ⇒ **Prefer 2′ over
2** unless the research finds a genuinely ignored bit *and* proves nothing validates entries it
did not write.

⚠ **Hierarchical detection would save the read** — read the small directory level, learn which
leaves changed — but only if a parent entry is rewritten when a child changes. Editing a PTE does
not normally touch its PDE. **If that holds, this route cannot save reads at any granularity.**

### ⊘ Polling a marker through the MMIO mapping does not help

`[measured, this tree]` word-at-a-time reads of video memory run at **13–15 MiB/s** (~0.3 µs a
word) against 48 MiB/s bulk. One marker per page is 1872 serialised reads ≈ **0.5 ms**, the same
order as CE-copying all 7.3 MiB (~0.8 ms) — and it **does not batch**, where the CE version is one
submission. ⇒ Same cost, batching given up.

### ★★★ Idea 3 — move the scan to the data

**Owner, 2026-09-14:** *"we upload a small ptx program that reads the vidmem with GPU speed to see
if there is an update and only reports back whats changed."*

| scanning 24 MB of tables | one pass | × 1178 refreshes |
|---|---|---|
| CE copy over PCIe @ ~10 GB/s | 2.4 ms | **2.8 s, ~28 GB of link traffic** |
| GPU kernel over vidmem @ ~360 GB/s | **67 µs** | **79 ms, ~0 link traffic** |

Shadow copy in vidmem; a kernel compares live against shadow and writes changed indices to a small
output buffer; CE back only that buffer. ★ **The link leaves the path entirely.** Composes with
idea 1 rather than competing: the invalidate says where to look, the kernel says what changed.

⚠ **Cost, stated honestly:** our own GR channel, our own module, our own launch path, and a
PTX/CUDA dependency on the host side — more new surface than ideas 1, 2′ and 4 combined. Not
foreign territory (guest compute forwarding works, `CUP3_VAL=43`), but *our* kernel on *our*
channel is not the same thing as forwarding the guest's.

### Idea 4 is a precondition of every design that CE-reads tables

Per-page submission is 1872 × 1178 ≈ **2.2M** doorbell/semaphore round trips; at even 20 µs each
that is **~45 s a boot — worse than the CPU path it replaces.** Batched it is 1178 submissions and
the cost vanishes into the bytes. ⇒ **Gather into one push, always.**

## ⚠ THE MEASUREMENT THAT ARBITRATES ALL OF THEM — and it has not been taken

**How many page-table pages actually change per refresh?**

★ Ideas 1, 2, 2′ and 3 all pay off **in proportion to sparsity**, and all are pointless if changes
are dense — then the bytes must cross regardless and batched CE (4) is the entire answer.

⊘ It is cheap and needs no new machinery: the refresh already re-reads the tables, so hashing each
page and counting changes is a small patch on the existing boot path. **One boot answers it.**
⇒ Take this number before building any of 1, 2′ or 3.
