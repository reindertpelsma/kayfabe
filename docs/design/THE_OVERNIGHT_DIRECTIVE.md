---
name: overnight_directive_2026_09_13
description: "★★★★★ THE STANDING OVERNIGHT DIRECTIVE (owner, 2026-09-13) — the ten goals, the rules of engagement, and the constraints. Governs until the work is done."
metadata:
  node_type: memory
  type: project
  date: 2026-09-13
---

# ★★★★★ THE OWNER'S OVERNIGHT DIRECTIVE — 2026-09-13

> *"See whats achievable."* Owner went to sleep; this governs autonomous work until done.
> ⚠ **This must persist across compactions.** It is the goal list, not a summary of one.

## The goals, in the owner's own order

1. **Get Blackwell to boot.**
2. **Raw client under the trap constraints**: NO read traps at all; write traps ONLY in BAR0,
   and NOT in PRAMIN; nowhere else (not BAR1, not BAR2).
3. **No blocking calls on the vCPU thread**, and no locks that hold it.
4. **The new DoorbellTable properly working** (`crates/kayfabe-device/src/dbtable.rs`).
5. **Code rot cleaned up, or MARKED with a plan** so the cleanup is mechanical. The eventual
   code targets PRODUCTION.
6. **Every remaining write trap sub-millisecond**, via the proper queue/defer model.
   ⊘ The ONLY accepted excuse for an overrun is time the thread was not scheduled and that is
   genuinely not ours — a vCPU steal counts; *"not scheduled because it was waiting on a
   blocking lock held on the vCPU thread"* does NOT.
7. **Raw client passing the MEAN test.**
8. **TWO raw clients with the mean test IN PARALLEL.** Ensure the worker/isolate concurrency
   plane works, and that MORE cuda ioctls/work can execute in parallel than there are workers
   in the main process (isolate threads combined) ⇒ **an epoll loop in the workers is required.**
9. **The LLM working**, then **the LLM at parity** under the same constraints.
10. **More CUDA apps that `nvkvm-pv` also passed**, and then **all of the above on Blackwell**.

## Rules of engagement

- **Subagents allowed.**
- **Renting vast boxes allowed — DESTROY them after use.** (`yes y | vastai destroy instance
  <id>`, then VERIFY with `vastai show instances`.)
- ★★★ **BEFORE declaring stuck, or stopping to discuss: ASK FABLE** (spawn an agent with
  `model: "fable"`). Use fable's solution. Only stop if fable reaches the SAME conclusion.
- **If I stop, DELETE the loop.** Never leave a stale useless loop running.
- **Commit and push to GitHub regularly.** `origin` = github.com/reindertpelsma/kayfabe.
- ⚠ **ALL code changes stay LOCAL** — vast storage is unreliable and has already cost this
  campaign a box mid-boot.
- ⊘ **DO NOT TRUST THE VAST BOXES.** Never pull executable code back from one. Never put
  secrets on one.

## ⚠ State at handoff

- **8 commits ungraded** (everything after w553). Raw client last passed at w553 on a
  16-core 3090 box that no longer exists.
- Bench: instance **50827717**, RTX 3090, 32 cores, `vastai/kvm:ubuntu_cli_22.04-2025-11-21`.
  ⊘⊘ THREE earlier rentals died because I used the image `vastai/kvm`, whose `latest` tag
  DOES NOT EXIST — `status_msg` said so and my restart loop kept clearing the field before I
  read it. **A create returning `success: False` is the answer; stop there.**
- ⚠ The box is a 3090 (GA102) and we present GA106. Under the owner's own matching rule
  (*"the GPU the guest sees must exactly match the one on the host, only driver version may
  drift"*) that is unsupported. A grade there is *"unchanged vs the last baseline"*, NOT
  *"passes"*. Say so.
- Read surface: 3839 of 4096 BAR0 pages backed. Remaining: 256 PRAMIN + 1 counter page.
- A subagent is analysing the any-architecture port → `docs/design/porting_to_any_architecture.md`.

Related: [[destroy_vast_box_50817052]] · [[provisioning_a_fresh_bench_box]] · [[the_three_blocking_invariants]]
