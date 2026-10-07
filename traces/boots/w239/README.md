# w239 boots — the spawn violation is GONE, and the next one is a different animal

★★★★★ **The pull fix works, measured.** Both logs, source revision `842c5c4`:

| file | `sandbox spawn` violations | `issuing a host RM verb` violations |
|---|---|---|
| `run_w239off_842c5c4_rvm_qemu.log` | **0** | 1 |
| `run_w239bt_842c5c4_bt_qemu.log` (`RUST_BACKTRACE=full`) | **0** | 1 |

⇒ At `810368b` the first log line was `spawning a sandboxed child process while holding
rank(s) [0]`. It is **gone**. The boot still aborts, on a **different** blocking call at the
same rank:

```
R1 no-blocking-under-lock violation: issuing a host RM verb while holding rank(s) [0]
```

Path (from the `_bt` log — `RUST_BACKTRACE=full` in QEMU's env, per `WORKFLOW_STRATEGY.md`):

```
RegPlane::write                    ★ state.lock() = rank 0
 → GspFsm::mmio_write_with → service_command_queue
 → ControlCensus / StickyAnswerGuard / PolicyChain::respond
 → ObjectPolicy::respond → Bridge::deliver
 → SharedObjectModel::forward_engine_object
 → SharedDevice::forward_engine_object_by_parent
 → Worker::execute                 ★ host RM ioctl round-trip
```

## ⊘⊘⊘ REFUTED 2026-08-11 (§16.96) — the section below is WRONG

`forward_engine_object`'s result is **discarded** at its only production call site
(`kayfabe_rmrpc::Bridge::deliver`: `let _ = gpu.forward_engine_object(…)`), deliberately, under
a paragraph that says *"the guest's answer does NOT change, and that is a decision, not an
oversight"*. ⇒ **nothing in the guest's reply depends on it**, it IS fire-and-forget, and it
was latched with §16.91's own pull pattern — no relocation, no reply memo, no third
`CommandPolicy` outcome. See `docs/design/execution_plane_increments.md` §16.96 and
`traces/boots/w244/`.

★ The error class: the ruling below was reached by reading a **signature**. A signature bounds
what a function *can* return; only the **call site** says what is read, and the type system
cannot see a `let _ =`.

## ⊘ Why deferral does NOT generalize to this one — ⊘ SUPERSEDED, see above

`forward_engine_object` returns `Result<EngineObjectForwarded, FwdFault>` — **a value the
guest's reply is built from**. The spawn was **fire-and-forget**: nothing in the response
needed it, which is exactly why it could be latched and drained later. This verb's result *is*
the answer. ⇒ **It cannot be deferred, only relocated**: the plane's lock must not be held
across the command policy at all.
