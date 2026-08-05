# `phys` is three different things, and one of them is host-physical

`[src]` 2026-08-05, at `e364608`. Written because the owner's directive —
*"every time you see `phys`, think GPGA or GPA; real phys doesn't exist in our project"* —
was about to be implemented as a mechanical `phys: u64 → Gpga` rename, and a classification
pass found that **the rename is wrong at two of the highest-traffic sites**.

## The three kinds

| kind | what it is | ours? |
|---|---|---|
| **GPGA** | GPU-physical: a framebuffer offset as the GPU sees it | ✔ |
| **GPA** | guest-physical: what the guest calls RAM | ✔ |
| **HPA** | host-physical | ✘ — unprivileged isolate, no access, and none needed |

## ⊘ Why a blanket newtype would be a LIE, at two named sites

**1. `kayfabe_mmu::Binding::phys` is APERTURE-DEPENDENT.** Its own doc already says so:
*"interpretation depends on `aperture`; for sysmem this is a guest-physical address."* So it
is a **GPGA** under `Aperture::Vidmem` and a **GPA** under sysmem. Typing it `Gpga` would
assert something the value does not have — which is precisely the `#170`/`#171` species
(*"a GPA-typed field holding a VA is the whole bug"*), re-committed one layer along.

⇒ this site does not want a rename. It wants an **aperture-tagged address** — one value that
cannot be read without its aperture. That is a design change to the address table's core
type, not hygiene, and it should be costed as one.

**2. `kayfabe_linux_raw::memtype::recorded_memtype(phys)` is GENUINELY host-physical**, and
legitimately so: it parses the host kernel's own `debugfs` PAT record for a host range. It is
**host-side introspection**, not an address this port stores, forwards, or hands to a GPU —
and the rustdoc directly above it already carries the guard rail
(*"Nothing in this port does that, and nothing may start without saying so here"*).

⊘ Read as a **declared exception** to the directive rather than a violation of it — but the
directive is the owner's, so the exception is theirs to confirm. Flagged in
`open_questions_for_the_owner.md`; not assumed here.

## ★ Why nothing was renamed tonight

A tree where *some* `phys` carry a type and others do not, with no rule a reader can apply,
is **worse** than uniform `u64` plus a stated rule: it invites the reader to trust the
absence of a type as information, when it only means nobody got there yet. Either the
conversion is whole or it should not start.

⇒ the work is **three items, not one**, and only the first is mechanical:

1. **`Gpga` for the unconditional sites** — the framebuffer window (`FbRead::read`/`write`,
   `FbRefused`, `Worker::fb_read`) and the page-table plane (`PtPage::phys`,
   `Spine::pt_roots`/`pt_learned`/`pt_contested`, `pt_page_owner`, `publish_pt_pages`,
   `ReachShadow::witness`). One closed interface each, so each converts whole.
2. **`Binding`'s aperture-tagged address** — a design change; needs its own increment.
3. **`memtype.rs`** — keep `u64`, and say *host-physical, by declared exception* at each of
   the four sites, once the owner confirms.

⚠ Until (1) lands, the rule is the directive itself and it lives in prose: **read every
`phys` as GPGA unless the surrounding plane is guest RAM, where it is GPA; never as HPA
except in `memtype.rs`.**
