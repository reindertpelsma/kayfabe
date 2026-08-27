# Workflow & cost strategy (agent sessions)

This repo's work is **long serial hardware debugging** (Mode-2 GPU bring-up): one GPU bench, fresh
QEMU boot per clean run, WPR2 resets only on restart, tests strictly serial. That shape makes agent
sessions long and token-heavy. This doc records how to keep them effective *and* economical. It is a
**convention doc**, not a rulebook.

## What actually costs tokens (measured 2026-06-13, ~20-day thread)

A single continuous conversation reached ~17.6 B token-events over ~45 K tool-loop API calls and 86
compactions. Breakdown of where the cost went, largest first:

1. **Iteration count (≈irreducible).** ~45 K build→boot→run→inspect cycles, each an API call that
   re-reads context. Hardware debugging is inherently thousands of cycles; this is the work, not waste.
2. **Per-turn fixed floor × N.** Everything that rides on *every* call — tool schemas, `CLAUDE.md`,
   and **`MEMORY.md`** — is paid once per call. A bloated `MEMORY.md` is the most expensive avoidable
   thing in the repo: every KB over budget is multiplied by tens of thousands of calls.
3. **Context ballooning between compacts.** Big tool outputs (25 MB logs, large file reads) accumulate
   in context until the next compact. The avg context/call ran ~387 K — far above the fixed floor.

~87% of the spend was **cache-reads** (re-reading accumulated context). Prompt caching already saved
~10× vs the uncached equivalent; the levers below cut what's left.

## Levers (cheapest-first)

- **Keep `MEMORY.md` under its size budget.** It's the per-turn floor. One line per memory, under
  ~200 chars, hook + link only; move detail into the topic file. Trim/consolidate aggressively.
- **Keep big outputs OUT of the main thread.** Don't read 25 MB logs into context. `grep`/aggregate,
  or hand the file to a **read-only subagent** that returns a one-page verdict. (The Fable log-traces
  this repo uses do exactly this — 25 MB log in, ~1 page out.)
- **Compact a bit sooner**, before context balloons — a well-timed compact ≈ a fresh conversation in
  what it re-sends (`system + tools + CLAUDE.md + MEMORY.md + summary + recent`), not the raw history.
- **Treat each debug episode as restartable-from-disk.** What *compounds* is the findings (refuted
  theories, root causes) — and those live in `memory/`, `docs/design/`, and commits, not in chat
  history. A fresh conversation seeded from memory + the saved host logs + committed code loses almost
  nothing. (Proven: agents reconstruct deep state from a saved log with zero conversation history.)

## Parallelism: serialize the bench, fan out the analysis

The core **cannot** parallelize — one GPU, fresh boot per run, serial tests (see
`memory/orchestration_model.md`). Don't fan out GPU runs; they wedge the device.

What **does** parallelize is the **read-only** half: log forensics, source tracing, RE, doc/spec
research. Run those as concurrent subagents *while* the single serial bench run is in flight, and have
each return only its conclusion. This is the standing model: **serial bench + parallel read-only
analysis**, hybrid not fan-out.

## Practical loop

1. Resume from `MEMORY.md` index + the relevant topic file + saved host logs (`/tmp/*.txt` on the
   bench host) — not from chat history.
2. Drive the serial GPU loop yourself (blocking ssh); spawn read-only subagents for tracing in parallel.
3. Write findings to memory/docs and **commit+push** at each milestone (durable state = the repo).
4. Compact (or start a fresh episode) before context balloons; keep `MEMORY.md` lean.
