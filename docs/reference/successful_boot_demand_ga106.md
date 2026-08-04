# What a boot that SUCCEEDS actually demands — GA106, measured

`[measured]` 2026-08-03 by decoding `traces/rpctrace_ga106_boot1.bin` (task #178's capture:
RTX 3060 GA106, open 580.159.04, `nvidia-smi` working) with
`scripts/rpctrace/decode_rpctrace.py --controls`.

This is the question the whole `replay-conformance` line was opened to answer: **how much is
left between where we stop and a working `nvidia-smi`, and what is it.** Until now the only
demand list we had came from `cap1`, a trace of a boot that **fails**, so it could only ever
be a lower bound.

## 1. The size of the remaining ladder

| | distinct controls |
|---|---:|
| `cap1_coldboot_hermetic` — the boot that **fails** (task #179) | **53** |
| `rpctrace_ga106_boot1` — the boot that **succeeds** | **104** |
| demanded by the successful boot, **absent from `cap1`'s list** | **53** |
| in `cap1`'s list, absent from the successful boot | **2** (`0x20810110`, `0x20810111`) |

⇒ **The ladder from here to `nvidia-smi` is 53 controls long**, and they are enumerated rather
than discovered one boot at a time. (The two numbers being both 53 is a coincidence of this
capture, not a relationship — 53 classified, 53 new.)

⚠ The two absent ids are `NV2081_BINAPI`, the pair #179 could not classify because CPU-RM
carries no export, flags, `paramsSize` or struct for any BINAPI control. Their absence here is
consistent with that: they are `nvidia-smi`-era enumeration in `cap1`, and this capture's
`nvidia-smi` took a different path through the tool.

## 2. ★★★ A real GSP REFUSES thirteen of them — on the boot that works

The most consequential row in the capture is not a reply, it is a **refusal**. Thirteen
controls come back with a non-`NV_OK` status from **real GSP firmware**, on a boot that goes on
to a working `nvidia-smi`:

| cmd | calls | status |
|---|---:|---|
| `0x2080012f` | 1 | `0x56` |
| `0x20800157` | 2 | `0x56` |
| `0x20800a87` | 2 | `0x56` |
| `0x20800b05` | 2 | `0x56` |
| `0x20801322` | 1 | `0x56` |
| `0x20801344` | 1 | `0x56` |
| `0x20801357` | 2 | `0x56` |
| `0x20809038` | 1 | `0x56` |
| `0x2080a0f2` | 1 | `0x56` |
| `0x2080a63c` | 1 | `0x56` |
| `0x90e70113` | 1 | `0x56` |
| `0x2080014b` | 10 | `0x0` **and** `0x57` |
| `0x20808546` | 18 | `0x0` **and** `0x56` |

`0x56` is `NV_ERR_NOT_SUPPORTED` (`ogkm-580: nvstatuscodes.h:115`) — **the same status this
port emits when nobody claims a command** (task #177, `gpfifo_schedule.md`).

Three consequences, and they change the plan rather than decorate it:

1. **We do not have to serve all 104.** Refusing with `NV_ERR_NOT_SUPPORTED` is *ordinary GSP
   behaviour* and the guest driver tolerates it. Eleven of these are unconditional refusals on
   this boot, so the ladder's *serve* obligation is at most 93, not 104.
2. ⊘ **But "refuse it" is not a free pass, because two of them are CONDITIONAL.**
   `0x2080014b` answers `NV_OK` on some calls and `0x57` on others; `0x20808546` answers
   `NV_OK` on some and `0x56` on others — over 10 and 18 calls respectively. A table that
   always refuses those is as wrong as one that always serves them. **The answer depends on
   arguments or on state, and a static policy cannot express it.**
3. ★ This is the first time we have had an **authoritative negative**. Every previous refusal in
   this port was our own decision, justified by reading. These are refusals *hardware itself*
   makes, on a boot that then succeeds — the strongest possible evidence that a given control
   is not on the critical path.

## 3. ⊘ What this still does NOT tell us

The `data`-vs-**ACT** distinction is unchanged and unanswerable from a trace
(`rpc_trace_capture.md` §4, and #179 measured it): `0x20800a6c` answers 17 bytes on some calls
and 49 on others, `0xa06f0103` answers 3 — **both look exactly like data in this table.**
Serving either from it fails *late*. The 53 new controls need the same static pass over `ogkm`
that #179 ran over the first 53 before any of them can be served from a capture.

Nor does a refusal here mean "refuse it forever": these are one boot, one board, one driver,
`nvidia-smi` only — no CUDA, no second process, no reorder.

## 4. ⚠ An error of mine, recorded because it is the night's own lesson

Before running the decoder I wrote **my own** parser for this file and reported to myself that
the successful boot demanded **187** controls. It does not; it demands **104**. My parser found
the `cmd` field by scanning for the offset that yielded the most plausible-looking control ids,
validated that on elements ≥ 160 bytes (258/258), and then applied it to elements ≥ 40 bytes
**without re-validating** — so on short elements offset 36 is not the `cmd` field at all and the
count inflated by 80 %.

The decoder I should have used was already in the tree, and had been **cross-validated 88/88
against a different instrument** (`traces/real_ga106/rpc_transcript_real_ga106.txt`). I had
committed the lesson *"before concluding an instrument is missing, check whether the project
already has one"* (`340eb07`) roughly an hour earlier, and then rebuilt a worse one anyway.
⇒ `suspect_the_instrument_first`, `gates_quantified_over_a_list` — **a universe extended without
re-validating the thing that made it work is a different universe.**
