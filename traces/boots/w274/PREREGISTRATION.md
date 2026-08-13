# w274 — PRE-REGISTRATION, written before the native capture is run

**STATUS: LIVE — 2026-08-12.** Written *before* `nvdp` is rebuilt or run. The read-of-committed-
artefacts half of this rung was done first (it cost no GPU time); everything below that is marked
**[already read]** was established from files already in the tree, and everything marked
**[to measure]** is a prediction recorded before the measurement exists.

⊘ `w273` did not pre-register and said so plainly. This rung *does* spend GPU time — a native
capture on `vh` — so it gets a real pre-registration, and the arms the brief named are all here
including the ones I expect to lose.

---

## The brief's premise, and where I already disagree with it [already read]

The brief asks: *"is this window SUPPOSED to be backed, and by what?"* — treating
`NVC7C0_SET_SHADER_SHARED_MEMORY_WINDOW`'s operand as an address whose absence from our page
tables is the wall.

I have already read three things that make me doubt the premise before measuring:

1. **The Xid names a different address family than the window.** `0x75b2_aee00000` (fault) vs
   `0x75b2_b9000000` (window) are **162 MiB apart**, and the guest's own `/proc/PID/maps`
   (`run_w271_pin_probe.log:166-174`) puts them in **two different reservations** with mapped
   records in between.
2. **The class header distinguishes `ADDRESS` from a `_WINDOW`'s `BASE_ADDRESS`**, and there is
   no `SET_SHADER_SHARED_MEMORY_A/B` at all.
3. **The faulting client is `HUBCLIENT_FE` doing `ACCESS_TYPE_VIRT_WRITE`** — the front end,
   writing. The native reference already records the front end writing exactly one thing in
   `cup2`: the I2M destination of `cuMemcpyHtoD`.

## ARMS — recorded before the capture

| # | arm | brief's weighting | my prior | what would settle it |
|---|---|---|---|---|
| A | **native has a backing for the window and we lack it** | the brief's main arm | **LOW** | a native `/proc/self/maps` record covering the window value with `r`/`w` perms, or an nvidia mmap window containing it |
| B | **the window is an APERTURE base, not a dereferenced address** — "inverts the rung" | the brief calls this the inverting arm | **HIGH** | native's window value also lands in an unbacked reservation, *and* the class header's naming holds |
| C | **MME TIED** — the 39 MME dwords produce the faulting address | inherited from w273, explicitly "not a measurement" | **LOW** | no cheap decisive test in this rung; I can only fail to exclude it |
| D | **MME EXCLUDED** — the faulting address has a fully-accounted non-MME producer | — | **HIGH** | native shows the same-shaped write with a plain pushbuffer literal, on the same channel |
| E | **`CUP2_RC` moves off 124** | low | **NO — untestable here** | ⊘ **this rung boots no guest.** `CUP2_RC` is not re-measured; the standing value is w271's `124`, and I will not restate it as a w274 result |
| F | **native `gpe[0]` emits `SET_SHADER_SHARED_MEMORY_WINDOW` at all** [to measure] | — | HIGH | the decoded native context-init segment |
| G | **native `gpe[0]` is 216 dwords, byte-comparable to the guest's** [to measure] | — | HIGH | already read off native's ring dump; the *contents* are not yet captured |
| H | **the fault address is `dp`, cup2's `cuMemAlloc` device pointer** | — | MEDIUM-HIGH | ⊘ **not directly measurable in this rung** — cup2 does not print `dp` and no boot is planned. It stays an inference with named support |

★ The brief warns that **six of the last seven rungs had their least-weighted arm fire**. My
least-weighted arms here are **A** and **C**. If either fires I will lead with it.

## What this rung CANNOT settle, stated in advance

- It cannot measure our guest at all — no boot. Every guest fact is a re-read of w271's committed
  logs at build rev `5feac90`.
- It cannot prove which producer emitted `0x75b2_aee00000`; it can only show that a producer with
  the right shape exists natively and needs no MME.
- A native run is **one workload, one chip, one driver**. It cannot speak for `cup3`+ or for any
  path that launches a kernel — `cup2` launches none.
