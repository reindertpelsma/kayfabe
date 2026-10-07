# w237 boots — route B wired, and the R1 violation that stopped both arms

★★★★★ **These two boots measure ONE thing and it is not route B**: an isolate **spawn runs
beneath the register plane's FSM mutex**, on the vCPU's own MMIO trap. w236 ranked that lock;
the first boot after it aborts.

| file | source revision | route B code | `KAYFABE_RING_VIDMEM` | R1 violations | outcome |
|---|---|---|---|---|---|
| `run_w236ctl_5626939_rankonly_qemu.log` | `5626939` (w236) | **absent** | n/a | **1** | QEMU aborted |
| `run_w237off_7107bba_routeb_off_qemu.log` | `7107bba` (w237) | present, **OFF** | unset | **1** | QEMU aborted |

⇒ **`RING-VIDMEM=0` in the first log is the provenance proof**: that binary predates route B
entirely, and it fails **identically**. Route B did not cause this.

The panic, verbatim from both:

```
R1 no-blocking-under-lock violation (l1_concurrency.md §3.3): spawning a sandboxed child
process while holding rank(s) [0] — a blocking call may only be made with ZERO ranked locks
held; drop every guard, round-trip on the checked-out worker, then re-acquire and RE-VALIDATE
```

Rank 0 is `LockRank::Plane`. ⇒ **`fork`/`exec` of a sandboxed child, under the lock every vCPU
takes for every register access.**

## ⚠ Provenance note — the tags in the raw filenames were WRONG

Both runs were tagged by a script whose suffix I edited by `sed`, and the w236 control kept
`7107bba` in its tag while being built from `5626939`. **The files here are renamed to their
true revisions**; the `_capture.log` files still contain the original (wrong) tag inside them,
and are kept unaltered rather than edited, because a doctored artefact is worse than a wrong
label with a correction beside it. The revision each binary really came from is established by
the `RING-VIDMEM` count above, not by either tag.
