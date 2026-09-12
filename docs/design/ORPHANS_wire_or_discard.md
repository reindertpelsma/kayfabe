# Orphaned code — wire it or discard it, no third option

**STATUS: LIVE, 2026-09-12.** ★ Three DISCARD rows acted on the same day — see the ✅/⊘⊘
markers in the table; two carried a factual error each, recorded in place. Owner: *"either planned to be wired and tested or discarded
later."* Every row below is code that exists, compiles, and has **no production caller**.
Each gets a decision, not a note.

⊘ **Why this is its own document.** `[surveyed w476]` the pattern is not incidental. Six
isolate verbs, one wire message with zero senders including tests, a complete doorbell table,
a correct reassembler that is unreachable, and 51 behaviour flags. Orphaned code is worse than
absent code: it reads as implemented, it passes review, and its tests go green while the path
never runs. This tree has already paid for that shape twice — `GraphPolicy` answers everything
and is test-only, and the #14 working-set gate has **never executed in a boot**.

## WIRE — these are correct and on the path we want

| what | where | why wire it | what wiring means |
|---|---|---|---|
| **`DoorbellTable`** | `kayfabe-device/src/dbtable.rs`, 316 lines, 9 tests green | ★ It is exactly the design the owner specified for the MMIO contract: one `u64` per token, two tag bits plus a 62-bit target, `route()` is a single relaxed load, no lock. Passthrough becomes one store, emulated becomes queue-and-wake, unallocated refuses. | Four arms on the vCPU write path, plus an installer on the worker at channel birth (the host token only exists after `GET_WORK_SUBMIT_TOKEN`). ⚠ The passthrough store is only one instruction if the host doorbell mapping was established off the vCPU first. |
| **`Reassembler` / fn 71** | `kayfabe-rmrpc/src/reasm.rs`; unreachable because `ObjectPolicy::respond_serviced` early-returns for `RmControl` then gates on `OBJECT_VERBS` | Real hardware sends **2 continuation records per boot** on all three captured boards. Our boots have never produced one, so nothing is red — the textbook "green test holding a wall in place". | Reach `Reassembler::accept` for fn 76 as well as 10/21/103. |

## DISCARD — built for a path that no longer exists

| what | where | why |
|---|---|---|
| `VerbPlan::Publish` + `alloc_sysmem` | `kayfabe-isolate/src/lib.rs:1908` | No production caller; `publish_backing` is reachable only from tests. Superseded by `JoinFbLeaf`. |
| `VerbPlan::PublishVidmem` + `alloc_vidmem` | `:1942` | Dead arm: its only producer is `fwd::backing_for`, whose callers are all in `tests/`. Production `how` is hard-set to `Joined`/`Aliased`. The shim says so itself: *"has no caller"*. |
| ✅ **DONE 2026-09-12** `export_device_view` + wire tag 25 | `kayfabe-isolate/src/lib.rs:3210` | **Zero senders of any kind**, tests included. The newest verb on the wire and the deadest. ⊘ **One caller the row missed, and it is not on the wire:** `rmladder --bar1-crossing` calls it on a concrete `HostRmBackend`, in-process. The trait verb, the `Worker` wrapper, `DeviceView`, `Request::ExportDeviceView` (tag 25) and `Reply::DeviceView` (tag 12) are deleted; the **host impl survives as an inherent method** on `HostRmBackend` so the bare-metal probe still builds. Tags 25/12 retired in place, never renumbered. |
| ✅ **DONE 2026-09-12** `export_surface` | backend verb 17 | No `Worker` wrapper exists; host impl is a stub returning not-implemented. ⊘ **The reasoning had a hole:** `present_seam.rs` reached it anyway, through `Worker::with_rm` — the escape hatch that is the row below. *"No wrapper ⇒ unreachable"* is not sound while a hatch around the wrapper exists. Verdict unchanged (both callers were tests). Deleted with its four impls, `RmVerb`/`VerbKind::ExportSurface`, request tag 12 and reply tag 6 (retired in place), and the two producer-only tests. `SurfaceHandle` + `Present` (the consumer half) kept. |
| ⊘⊘ **NOT DONE — the row's premise is wrong** `Worker::with_rm` | `:4081` | ~~Callers are a mock and an in-process ladder binary.~~ **Measured 2026-09-12: 22 call sites in 4 files** — 14 in `kayfabe-mocks`' own `#[cfg(test)]` module, 3 in `kayfabe-isolate-host/tests/guest_ram.rs`, 3 in `tests/real_isolate.rs`, 2 in `bin/rmladder.rs`. They reach `alloc_vaspace`, `alloc_sysmem`, `free`, `schedule` and `describe_guest_ram` — **all live production verbs**, and `Worker` has **no wrapper for any of them**, nor does `VerbPlan` have a single-verb plan that isolates one. So removing the hatch means deleting ~20 tests of live code, or adding five new public wrappers (growing the surface this document is pruning). ⇒ Decide the hatch and the missing wrappers together; it is not a delete. |

## DECIDE WITH A MEASUREMENT — not yet either

| what | why it is not a simple delete |
|---|---|
| `fb_read` | Called in production, against a host impl that unconditionally returns `NOT_ON_THIS_RUNG`. So the CALL is live and the IMPLEMENTATION is absent — deleting the verb would remove a real caller's only option. Decide by finding out what that caller needs. |
| `export_backing` | Test-only sender, but production `ExportedBacking` values come from the `JoinFbLeaf` reply instead. Likely discard, after confirming nothing external depends on the wire tag. |
| **The 51 behaviour flags** | Each is an experiment; the loser of a settled one is pure subtraction. ⚠ I added three today and one (`KAYFABE_PENDING_VIA_LOCK`) is already settled and should go. Needs an audit that separates settled from live, which is a mechanical question. |

## The rule that stops this recurring

★ **An arm is an experiment, and an experiment has an end.** When an A/B is settled, the losing
side is deleted in the same commit that records the result. A flag that survives its own
experiment is how 51 of them accumulate, and every one of them doubles the state space that
everything else has to be correct in.
