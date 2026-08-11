# w244 boots — the bench BOOTS again, and the verb that blocked it was fire-and-forget

★★★★★ **BOOTABILITY RESTORED, measured.** Source revision `acbb9a39579a0c796b19318fe9b4c3508f2367d9`,
**stamped inside the binary** and quoted in each probe log
(`=== archive rev STAMPED IN THE BINARY: kayfabe-rev:acbb9a3…`), never from `BUILD_REV.txt`.

⊘⊘ **THIS IS BOOTABILITY, NOT FORWARDING PROGRESS.** `CE-SUBMIT` is **0** in both arms and
nothing here may be read as the first forwarded compute. The bench could not boot at `5626939`
or later; it can now.

| | `sandbox spawn` violations | `issuing a host RM verb` violations | `RmInitAdapter failed` |
|---|---|---|---|
| `810368b` (w238, before §16.91) | **1** | — | — |
| `842c5c4` (w239, after §16.91) | 0 | **1** → QEMU aborts | — |
| **`acbb9a3` (w244, this rung)** | **0** | **0** | **0** |

Evidence (the three files `scripts/bench/assert_boot_evidence.sh` gates on) is in
`traces/guest_boots/run_w244{a,b}_acbb9a3_*_{qemu,dmesg,probe}.log`; the harness transcripts
are here.

## The two arms, and why both were needed

| arm | `KAYFABE_CE_EXECUTOR` | what it measures |
|---|---|---|
| `w244a_acbb9a3_ceh` | `host` | ★ **The differential.** Byte-identical configuration to w239's two boots, so the only variable is the code. The `issuing a host RM verb` count goes **1 → 0**. |
| `w244b_acbb9a3_cel` | `local` (the bench default) | The configuration the tree normally runs, so "the bench boots" is not a claim about one exotic switch. |

⊘ Both arms carry `RUST_BACKTRACE=full` **exported into QEMU's own environment**
(`WORKFLOW_STRATEGY.md`) — the instrument that found the violation in the first place is left
armed, so a *new* R1 path would print its whole transitive stack rather than a bare message.

## ★★★ What proves the DRAIN is wired, from the boot itself

`report_engine_forward` has exactly **one** caller — `report_engine_forward_drain` — which has
exactly **one** caller: `Regs::write`, *after* `RegPlane::write` returns and the plane's rank-0
guard is a dropped local. The admission path prints nothing at all except the latch-full
refusal.

⇒ **every `kayfabe: ENGINE-OBJECT … → FORWARDED/REFUSED` line in these logs was printed by the
lock-free drain.** `w244a` has **32** of them (`[seen=32 forwarded=18]`); `842c5c4` has
**one**, and the R1 abort follows it. ★ That is the one claim the type system could not make —
that the call site is on the path — answered by the bench instead.

## ⊘ What did NOT fire, and that is also the measurement

- `ENGINE-FORWARD LATCH FULL` — **0**. The bound (`MAX_PENDING_ENGINE_FORWARDS = 64`) never
  refused anything, i.e. the observed population stayed at the one entry the design predicted.
  ⚠ It is kept because that is *"a property of the guest, not of the protocol"*
  (`kayfabe-gsp/src/boot.rs:1291-1294`), and because it doubles as the **missing-drain
  detector**: a `Regs::write` that stopped draining would fill the latch and say so.
- `ENGINE-FORWARD DRAIN OVERRUN` — **0**. No drain reached the 1 s budget, so no guest wait was
  extended anywhere near `_kgspRpcRecvPoll`'s 6 s (`ogkm-580: kernel_gsp.c:2379` over
  `os.c:1978`; three timeouts mark the GPU for reset, `:2455`).

## ⊘ The standing wall is UNCHANGED, and that is the correct outcome

`w244a`'s `cup2`: `cuInit` ok, `devices=1`, `compute=8.6`, `totalMem=11959 MiB`, then
**`CUP2_RC=124`** — the 180 s timeout at `cuCtxCreate` that `scripts/bench/cup2_hook_w232.sh`
documents as *"the standing wall on both executor arms"*. Doorbell census: **191 arrived, 183
served, 8 REFUSED by name** — reproducing the `w229`/`w230` population exactly.

⇒ This rung removed a **regression in bootability** introduced by ranking the plane's lock at
`5626939`. It did not move the wall, and it was not supposed to.

⊘ `nvidia-smi` still prints `ERR!` in the Name column (`GPU_GET_NAME_STRING` returns zero
bytes) — the known, unrelated gap. `SMI_RC=0`.

Full record: `docs/design/execution_plane_increments.md` §16.96.
