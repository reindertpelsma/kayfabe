# Workflow strategy — debugging instruments

**STATUS: RESEARCH, 2026-08-11.** A one-entry note, written on 2026-08-11 from a pre-v3 lock-rank debugging session, on how to find a transitive lock violation with `RUST_BACKTRACE=full` in QEMU's environment. Read on 2026-11-07: the R1 lock witness it refers to still exists as `crates/kf-util/src/lockwitness.rs` and no script under `scripts/` sets `RUST_BACKTRACE`, which matches the note's "the default boot does not set it"; the example's sections and boot logs were not re-run.


## ★★★ Finding a lock violation whose call is TRANSITIVE — `RUST_BACKTRACE=full`

`[measured 2026-08-11, §16.89]` An R1 assert fired naming rank 0, and **`grep`ping the crate
that holds that lock for the offending call found nothing** — the call was **fifteen frames and
six crates** below the acquisition. Two people looked for it by name and neither found it.

**The instrument**: export `RUST_BACKTRACE=full` into **QEMU's own environment** (the boot
script's env, not the ssh session's), then read the panic in `run_<tag>_qemu.log`. The assert
prints the whole path, crate by crate, and the offending frame is unambiguous.

⊘ **The default boot does not set it**, and without it the log carries only the message plus a
stripped backtrace whose middle frames are inlined away — enough to know a rank was held,
useless for knowing by whom.

★ Reach for this whenever a rank/lock assert names a rank the local code does not take. A
six-crate transitive call is not greppable, and the next one will not be either.
