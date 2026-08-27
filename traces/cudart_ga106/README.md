# `cudart` host↔guest differential — GA106, 580.159.04, 2026-08-20 (w350)

**STATUS: LIVE.** Both traces are the SAME one-call program (`cudaGetDeviceCount`) recorded
through the same `nvdiff` `LD_PRELOAD` shim, which captures the parameter buffer on **both
sides** of every ioctl. `host_rt` was taken on the bare-metal GA106; `guest_rt` inside our
guest on the same box, same hour. Host answers `0`; guest answers **`3`**
(`cudaErrorInitializationError`).

⊘ **`strace` cannot see this class at all** — RM puts its verdict inside the parameter
struct, so every `ioctl(2)` returns `0` on both sides.

## What the pair establishes

| question | answer |
|---|---|
| record counts | host **105**, guest **104** |
| first divergence by `(dev, nr, cmd, status)` | index **98** |
| records differing on **status** | **2** — `0x2080a084`, `0x2080a026` (both `0x0` vs `0x56`) |
| records differing on **reply BODY** at equal status | **27** |

★★★★★ **The two status divergences are NOT the cause, and that is measured, not argued.**
An `LD_PRELOAD` interposer refused them on the bare-metal host, in-band exactly as we refuse
to the guest — individually **and jointly** — and the host's own `cudaGetDeviceCount` stayed
`0` in every arm, with the empty-set control run first and last.

⇒ **The cause is a reply we return with `status = 0` and the WRONG CONTENT.** That is the
`#203` defect class (an under-filled body decoding to zeros) and it is invisible to any diff
that compares vocabulary or status. **Diff the bodies.**

## Ranked leads — by KIND, never by index

Ranked on *"we returned a value where hardware returned a different real one"*, and
**excluding** rows whose difference is environmental by construction (pointers, UUIDs, the
GPU name string, per-process handles).

1. **`0x20800170` `GPU_GET_ENGINES_V2` — a COUNT and its MISSING ENTRIES.**
   `[0] host=9 guest=6`, and words `[7] [8] [9]` — engine ids `0x1b`, `0x22`, `0x33` — are
   words we **echoed from the request** (zero) while hardware **wrote** them. Three engines
   short. This is `numEntries` again, and it is the strongest lead on the board.
2. **`0x20801803` `BUS_GET_PCI_BAR_INFO` — four fields left at ZERO that hardware fills:**
   `[6]=0xc0000000`, `[13]=0x20`, `[18]=0x10000000`, `[19]=0x20`. BAR sizes.
3. **`0x20801303` `FB_GET_INFO_V2` — we return the SENTINEL `0xbadd00` TWICE** where
   hardware returns two *different* real values (`0xb8f180`, `0xba1740`). A placeholder
   reaching the guest as an answer.
4. **`0x2080a026` — 92 words echoed from the request** that hardware writes. ⚠ Its body
   carries **pointer-shaped** pairs (`[72]/[73] = 0x7002_96e1add0`); those are already in the
   *request*, so they are the guest's own and must never be replaced with captured host
   values.
5. **A `7` vs `3` family** across ten NV0000-class system controls (`0x201 0x202 0x205 0x214
   0x215 0x288 0x13a`). One quantity, ten sites. ⚠ Environmental status **unresolved** — do
   not act before deciding whether the bare-metal host's system config makes this a
   by-construction difference.

## Traps this pair already paid for

- ⚠ **A per-id refusal bisection finds INDIVIDUALLY sufficient causes and cannot find a SET.**
  Both `0x2080a084` and `0x2080a026` measured innocent alone; the guest refuses both at once;
  and even the pair measured innocent. Necessity and sufficiency are different experiments.
- ⚠ **Decode word counts from `paramsSize`, not from the hex string length.** A first cut used
  `len(hex)//8`, inspecting exactly **half** of every body and under-reporting nonzero words.
- ⚠ **A shim that fails to load produces an empty trace, which reads as "no ioctls".** The
  capture hook checks the loader and reports the record count separately.

---

## ⊘⊘⊘ ALL FIVE RANKED LEADS ABOVE ARE REFUTED — 2026-08-20, same box, no boot

The leads were ranked by *plausibility*, then tested by **substituting our value into the
working host** (`tools/cudart_probe/subst_shim.c`: run the real ioctl, then overwrite the
named reply word, so the caller sees exactly what our guest would have seen).

★ **Instrument proven before use.** Known-positive: corrupting `0x20809009` word[1] from
`0xd` to `0` turns the host from `0` into **`3`**. So a wrong VALUE at `status = 0` *can* kill
cudart and this shim *can* produce it. Every "innocent" below is therefore a real negative,
not a silent no-op.

| lead | injected | host verdict |
|---|---|---|
| 1 `GPU_GET_ENGINES_V2` count 9→6 + zero the 3 entries | yes | **0** innocent |
| 2 `BUS_GET_PCI_BAR_INFO` four fields → 0 | yes | **0** innocent |
| 3 `FB_GET_INFO_V2` → our `0xbadd00` sentinel | yes | **0** innocent |
| 1+2+3 together | yes | **0** innocent |
| **ALL 136 non-identifier divergences at once (241 substitutions)** | yes | **0** innocent |

⊘ And lead 1 was never a bug to begin with: `deviceinfo.rs:98` **deliberately** omits
NVDEC0/NVENC0/OFA/SEC2 with the reason written down — *"an engine we advertise is an engine RM
goes on to use."*

⇒ ★★★★★ **THE WALL IS NOT IN THE RM REPLY CONTENT AT ALL.** Not the vocabulary (98 records
identical), not the statuses (2 divergences, innocent alone and jointly), not the bodies (all
136, injected together). Whatever kills guest cudart is **outside the ioctl reply stream**.

## ★★★ THE PRIME SUSPECT IS NOW THE ONE THING I EXCLUDED

The `7`-vs-`3` family across `0x201 0x202 0x205 0x214 0x215 0x288 0x13a` is the **only**
injection that changes the host's verdict — to **`100` `cudaErrorNoDevice`**, after which
cudart exits and no later rule fires.

⚠ I excluded it as *"an identifier, so environmental by construction."* **That was an
assumption, not a measurement**, and it is now the only surviving candidate. The question it
raises is sharp and testable:

> Is the guest's `3` the guest's own consistent identifier (benign), or a value we report
> that disagrees with what our own guest driver expects (a real defect)?

⊘ Note the instrument's limit, which this exposes: **substituting an IDENTIFIER cannot
distinguish "our value is wrong" from "our value is a different valid identifier"** — the host
fails either way, for reasons that may have nothing to do with our port. The identifier family
needs a different experiment than the value family.

## Other unexplored residue

- The host issues **105** records, the guest **104**. One call the guest never makes — untested.
- `mmap` results, `/proc` and `/sys` reads, and UVM behaviour are **outside this trace pair**
  entirely.
