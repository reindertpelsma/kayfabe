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

## ⊘ Why deferral does NOT generalize to this one

`forward_engine_object` returns `Result<EngineObjectForwarded, FwdFault>` — **a value the
guest's reply is built from**. The spawn was **fire-and-forget**: nothing in the response
needed it, which is exactly why it could be latched and drained later. This verb's result *is*
the answer. ⇒ **It cannot be deferred, only relocated**: the plane's lock must not be held
across the command policy at all.
