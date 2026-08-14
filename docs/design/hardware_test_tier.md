# The hardware test tier — how a test that needs a real GPU (or a real VM) is run

> ### STATUS — 2026-08-14 (w295) / **LIVE**
>
> Built and measured this rung on a real GA106 (`vh2`, RTX 3060). The `GPU-GATE` half is
> **done and green**; the guest-VM half is **deliberately not automated** and §4 states why
> and what would change that. ⊘ Supersedes the line at
> `docs/reference/full_suite_on_real_hardware.md:64` — *"No test anywhere gates on a real
> GPU"* — which was true on 2026-07-30 and false from the day `e6_hw_join.rs` landed.

## 0. The ask

> *"i would definitely add the raw client we have now in a test suite. maybe we should write
> some tests now to cover some bugs we had, its fine if some tests are real vmm/gpu only
> (maybe think of easiest way to execute test suite with real vm boot)"* — owner, 2026-08-14

## 1. The answer, in one line

**There is no new mechanism.** A hardware test is an ordinary `#[test]` that calls a gate
function, and `cargo test --workspace` is the whole command:

```sh
cargo test --workspace          # on a GPU box the GPU-gated tests EXECUTE
                                # elsewhere they print GPU-GATE: SKIPPED and assert nothing
```

There is **one** selection mechanism, not three: no cargo feature, no `KAYFABE_GPU_TESTS=1`,
no separate binary. This is a deliberate choice against the two alternatives:

| alternative | why not |
|---|---|
| a cargo **feature** | a feature that is off by default is a test that nobody runs; a feature that is on by default fails to compile on a box without the dependency. Neither can be *counted* — a feature-gated-out test is absent from the binary, so no reached-count can see it |
| an **env var** | the tree already has one (`KAYFABE_SLOW`) and it is the right shape for *"this is slow"*. It is the wrong shape for *"this needs a device"*: the honest question is not what the operator asked for, it is what the box **has**, and only a probe answers that |

## 2. The gate, and the one rule that makes it worth having

```rust
fn gate(test: &str) -> Option<Arc<RmConnection>> {
    // … RmConnection::open(&dev, GPU, pinned_host_classes())
    Ok(c)  => { gate_line(&format!("GPU-GATE: RAN {test}"));      Some(Arc::new(c)) }
    Err(e) => { gate_line(&format!("GPU-GATE: SKIPPED {test} — … asserts NOTHING here")); None }
}
```

Three properties, each paid for elsewhere in this tree:

1. **The probe is the real thing.** `RmConnection::open`, not `[ -c /dev/nvidia0 ]`. A box can
   carry the device node and no usable driver; a probe that cannot separate those reports the
   wrong kind of zero. (`run_full_suite.sh`'s own `probe_gpu` *is* the `stat` — correct there,
   because it is choosing whether to spend minutes, not deciding what a green means.)
2. **Both arms print, to the raw `stderr` descriptor.** `libtest` captures a test thread's
   output and flushes it only on **failure**, so `eprintln!` is invisible on exactly the runs a
   count needs. `[measured 2026-08-03, rev a1cdfdd]` `grep -c "GPU-GATE: RAN"` over a full
   `cargo test --workspace` was **0** while the gated test passed against a real GA106.
3. ⊘⊘ **A SKIPPED ORACLE KILLS THE GUARD.** A hardware test that skips silently is *worse than
   absent*, because the suite then reports green over it. So the skip is a line, and the line
   is counted twice, by two mechanisms that cannot silence each other:

   | mechanism | what it catches |
   |---|---|
   | `run_full_suite.sh`'s `gate-census` | on a `gpu-box` **profile**, a single `GPU-GATE: SKIPPED` is a **red run** — the box promised the capability and did not deliver it |
   | `ci.yml`'s `GPU-gate reached-count`, floor **3** over `RAN + SKIPPED` | the family quietly **shrinking to nothing** — a hardware test deleted or de-announced, which from GitHub looks green and one number smaller |

   ★ The floor is over `RAN + SKIPPED`, never `RAN`. GitHub has no GPU; on that runner the
   family is counted and never passes, and that is the point.

## 3. The raw client, and why its verdict is four assertions

`crates/kayfabe-isolate-host/tests/raw_ce_client.rs` is R33 arm 1 as a test: `alloc_vaspace` →
`prove_ce_copy`, the same `HostRmBackend` the ladder builds, no `libcuda`.

⊘⊘ **It never collapses to one boolean.** The client's banner states the bar as four facts;
`[measured 2026-08-13, boot `w283c_client`]` its verdict implemented three and printed
*"GP_GET 0 caught GP_PUT 1"* on its **★ success** line. So the test asserts, separately:

| | fact | why separably |
|---|---|---|
| 1 | the destination did **not** already hold the answer | otherwise nothing below is evidence |
| 2 | first word **and last** carry the source's bytes | a truncated copy is a different defect from a failed one and must not be reported as one |
| 3 | the semaphore carries the **declared** payload | bytes without our release are not attributable to this submission |
| 4 | `GP_GET` reached `GP_PUT` | that cursor is **this channel's own USERD** — a path that executes the work on a *different* host channel cannot advance it, which is exactly the state a forwarding bug produces |

The conjunction (`met_the_whole_bar()`) is asserted **last**, as a cross-check: if it fires
while all four rows passed, the instrument disagrees with itself, and the message says so.

**Measured, this rung, real GA106:**

```
GPU-GATE: RAN the_raw_ce_client_moves_bytes_and_each_of_the_four_facts_holds_on_its_own
GPU-GATE: R33 ACCEPTANCE all four facts hold separately — 4096 bytes,
          src 0x0000000120000000 dst 0x0000000120010000,
          semaphore 0x00000001 == declared 0x00000001, GP_GET 1 == GP_PUT 1
```

`run_full_suite.sh` also gained an 18th phase, `rm-ladder-ce-client`, which runs the **binary**
(`--ce-client`) and greps its own verdict line, **anchored**. ⊘ The two existing `rm-ladder`
phases assert nothing but the exit code — which is precisely the door `w283c` walked through.
Two independent readings of one fact, so a future re-collapse has to defeat both.

## 4. ★★★ The guest-VM tier — what it costs, and why it is NOT a phase

**A boot is minutes, GPU tests run strictly serially, and the machinery is complete but manual:**
`scripts/bench/boot_capture.sh` boots and captures with six distinct failure exit codes;
`scripts/bench/assert_boot_evidence.sh` is the one exit-status gate over a committed boot;
`POST_CAPTURE_HOOK=…/r33_hook_ce_client.sh` already pushes a **musl** `kayfabe-rm-ladder` into
the guest and appends `R33_RC=`.

⊘ **It is deliberately not wired into `cargo test` or into a `run_full_suite.sh` phase**, and
the reason is not effort:

* **Batching beats phasing.** One boot that checks twenty things beats twenty boots. A test
  *per assertion* is the wrong granularity when the fixture costs three minutes — it would
  either boot per test (unaffordable) or share a boot behind a `OnceLock` (a shared mutable
  fixture across libtest's threads, which is the shape every flake in this tree came from).
* **The artefact already exists and is durable.** 434 files under `traces/guest_boots/` and 58
  rung directories under `traces/boots/`. ★ *Prefer asserting against a committed log the boot
  produces over re-deriving state* — a boot's evidence outlives the box it ran on, and a rented
  box does not.
* **The gap is grading, not booting.** `assert_boot_evidence.sh` grades **capture
  completeness** and says so (*"a red boot is evidence too"*). The scripts that grade *results*
  — `w264…w269_grade.sh` and the inlined blocks in `w270…w294_run.sh` — **have no exit status**;
  `w269_grade.sh` never sets `rc` at all. So a boot's numbers are graded by a human reading
  rows, every rung, freshly.

⇒ **The next increment, stated so it is not re-derived:** make the grader a **pure function over
log text** in Rust, exercised off-bench against the committed corpus (which gives it
known-positives *and* known-negatives with no GPU at all), and have the fresh boot feed the same
function. That is the shape that turns a boot into a test. It is not in this rung because the
census showed the *harness* defects — §5 below — were cheaper and were causing false results
today.

## 5. What this rung guarded instead, and why

`tests/tests/bench_harness_gates.rs` — the bench harness, gated from outside it.
`gate_runner_floor.rs` and `full_suite_ledger.rs` do exactly this for `ci_gates.sh` and
`run_full_suite.sh`; **the defects all happened in `scripts/bench/`, where no such gate
existed.** Three rules, each from an incident that produced a *false result*:

1. a `CUP2_RC` **verdict** read must be anchored (the guest **compiler** reports
   `GCC_CUP2_RC=0`, first, and `grep -m1` returns it);
2. a boot tag must be caller-influenceable (an unconditional tag made boot 3 overwrite boot 1's
   logs **while printing a perfect result under boot 1's filename**);
3. no statement may follow a script's terminating `exit` (a grading block was committed below
   one, twice — and it *prints nothing*, which is what a grading block does when there is
   nothing to report).

★ Each rule is a **pure classifier with its own fixtures**, asserted against literal lines
quoted from the tree *before* it is pointed at the tree. A scanner run only over a clean tree
returns clean, and clean is exactly what a broken scanner returns.

## 6. The inherited red set, measured — not inferred

Measured at **`origin/master` = `72f902f`**, clean clone, clean target dir, on `vh2`
(RTX 3060 / GA106), and again at this branch's head. **This rung adds none of them and fixes
none of them** — they are stated so that a red run is attributable.

```
cargo test --workspace --no-fail-fast   →  EXIT 101, 5 targets failed, IDENTICAL both times:
  kayfabe-isolate-host  executor_vas_census
  kayfabe-isolate-host  guest_ring_census
  kayfabe-tests         ce_representability_split
  kayfabe-tests         doorbell_reaches_the_completion_observer
  kayfabe-tests         ring_out_of_our_own_framebuffer
```

⊘ **And two more that `traces/w294_cudalimit/README.md` §8 does not name, because it counted
`cargo test` targets only:**

| gate | at `72f902f` | at this branch |
|---|---|---|
| `cargo fmt --all --check` | **RED — 21 files** | RED — the same 21. This rung's two new files were formatted; the pre-existing 21 were left alone, deliberately: reformatting files a sibling lane is editing is a merge conflict wearing a cleanup's clothes |
| `cargo clippy --workspace --all-targets -- -D warnings` | **RED — 5 errors** | RED — the same 5, at `proto.rs:118`, `rmgraph.rs:584`, `rm.rs:1619`, `rm.rs:2600`, `rm.rs:6780`. None is in code this rung wrote |

⇒ The two CI steps that the README calls "clean" have been red on `master` for at least this
rung's lifetime. ★ That is worth its own line because **`ci_gates.sh --all` runs both**, so the
authoritative local gate run cannot currently reach a clean exit for a reason that has nothing
to do with any rung's change — which is the *"a red caused by the box cannot be reported as a
red caused by the code"* rule, one category over: **a red caused by the INHERITED TREE must not
be reported as a red caused by the branch.**
