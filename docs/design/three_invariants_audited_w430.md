# Three invariants, audited against master — 2026-09-11

Owner asked three yes/no questions. Answered by source census on `master`, not from memory.
**One yes, two no.**

## 1. VAS publish at the doorbell — ✅ DEAD

`publish_vas_rows` has exactly **two** call sites in the whole tree
(`shim.rs:5107`, `shim.rs:5210`). Both are inside `doorbell_publish_loop`, which is spawned as
its own thread (`shim.rs:13828`) and mints `OffVcpu::for_publication_worker()`. There is no
call site on a vCPU.

`refresh_page_tables` likewise has one call site (`shim.rs:5189`), in the same worker.

⊘ And the doorbell itself no longer even reaches that code inline: `DoorbellAsyncArm::from_env`
maps `None | Some("on")` to `On`, so with the variable unset the default **defers** — `ring()`
offers to the publication queue and returns. `ring_inline` is the `off` arm only.

## 2. BAR1 / BAR2 untrapped — ⊘ NO. BOTH ARE TRAPPED ON MASTER.

`qemu/hw/misc/nvkvm/nvkvm.c:1119-1123`: every non-MSIX row is built with
`nvkvm_region_init_io` and registered with `pci_register_bar` — pure MMIO, every access traps.

The passthrough exists but is **default-off**:

    nvkvm.c:3498   DEFINE_PROP_BOOL("bar1-passthrough", NvkvmState, bar1_passthrough, false)
    nvkvm.c:3500   DEFINE_PROP_BOOL("bar2-passthrough", NvkvmState, bar2_passthrough, false)

The only thing that arms them is `scripts/bench/w393_bar_boots.sh`, the dedicated A/B
experiment. **Every ordinary boot — the LLM boot, the trigger boot, the R34 boot — runs with
BAR1 and BAR2 trapped.** The w393 measurement (BAR1 traps 88 193 → 91, client `(P)` on all
arms, n=2) lives on branch `w393-bar-passthrough` @ `fa9d6395` and has never been merged;
the merge decision was left open.

## 3. No heavy inline blocking on a vCPU — ⊘ NO.

`kayfabe-rt/src/device.rs:2663`:

```rust
let off = kayfabe_util::trapwitness::OffTrap::at_a_host_verb(
    "kayfabe_rt::SharedDevice::verb_op — the execute phase",
);
let executed = worker.execute(&verbs, &off);
```

`at_a_host_verb` takes, in its own words, *"the honest branch: a `claim` on the publication
worker, a **counted** `inline_under_bql` on a vCPU inside a guest trap"*. So when a guest trap
drives a host verb, the **real RM ioctl round-trip runs on the vCPU, under the VMM's global
lock**, and is counted rather than prevented.

`[measured w394, commit bb421ddf]` `inline_exceptions=166`, `INLINE-BY-REASON [134 ×
SharedDevice::verb_op execute phase] [32 × fwd::dispose_on]`, `off_trap_claims=0`,
`worst_trap=1 771 955 us`. The mechanism is unchanged since; the count wants re-measuring on
the new box.

⇒ The planned fix — moving `verb_op`'s execute phase to the `OffTrap::claim` branch so host
verbs run on the worker — is **not done**.

## ⊘ Found while auditing: a log line that OVERSTATED a violation

The invalidate lane's `Offered::Full` arm printed *"publishing INLINE on the vCPU and
completing here … a boot that prints this has a vCPU held for a publication"*. The code under
it does no such thing: it increments a counter, arms a full rescan, and returns. Corrected.

⚠ A log line that overstates a violation is as harmful as one that hides it — this one would
have sent a reader to audit a path that is already clean, and to distrust the vCPU-blocking
census that is already correct.
