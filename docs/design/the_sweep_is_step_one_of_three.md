# ⊘⊘⊘ The doorbell sweep is STEP 1 OF 3 — and the C's own analysis calls a DIFFERENT step "the operative one"

**STATUS: LIVE — 2026-08-12 (w275, written at a redirect, before any code was written).**
This doc exists to stop a lane from building step 1 of a three-step sequence while skipping the
step the already-committed answer names as decisive, and while skipping a **cheap priority-1
audit**.

## The redirect

A redirect ruled: *"The C's `cuCtxCreate` returned because it enforced an invariant we never
ported"* — a **doorbell-time sweep** of the guest's GR VAS page tables
(`nvkvm_gpu_emul.c:582`, *"Fault-safe: a mapping is always backed before the engine that uses it
runs"*) — and *"every fault this campaign chased one at a time is one instance of that single
missing invariant."*

## ⊘ The contradiction: this question was ANSWERED TWO DAYS AGO, as the owner's own question

`docs/design/how_the_c_passed_the_gr_wall.md` — **STATUS: answered 2026-08-10, owner's
question**, title *"How the C passed the wall we are standing on"*. It is verified against git
ancestry (`ceb13f5` proven to be in the binary that produced `cap3`).

Its **§4, "What the C did at OUR exact address"** — our address, `0x2_0440fff0` — lists the C's
actions **in order**, and the sweep is **first of three**:

1. **Host-backed the pool page** — *"compute-aperture (`VA ≥ 0x2_0000_0000`) working-set sweep
   pins pushbuffers **and the pool** into the GR fvas via `back_and_map_sys`."*
   ⇒ **This IS the redirect's invariant.** It is not unknown to this project; it is step 1.
2. **Resolved the GR VAS PDB** — `chan_own_pdb` fallback, `populate_cvas` retry-until-resolve.
3. ★★★ **`GPFIFO_SCHEDULE`d the GR TSG** — *"cont.34 FIX B, **the operative one**"*, one-shot per
   `(client, tsg)` in `nvkvm_m2_exec_doorbell` (`:4176-4194`, `:8038-8048`):
   > *"Without it the 8 GR-family channels ring (`GP_PUT=1`) and the host never consumes
   > (`gp_get` stuck 0). **With it, `gp_get` advances 0→4 and all 16 pool semas advance →
   > `cuCtxCreate` returns.**"*

⇒ **The doc calls step 3 operative, not step 1.** A lane that ports only the sweep is porting the
step its own source does not credit with the outcome.

## ⊘ And §6 is a RANKED WORKLIST that puts the sweep nowhere and an audit first

`how_the_c_passed_the_gr_wall.md` §6, *"What to do, in priority order"*:

1. ★★★ **Audit our writer set at `0x2_0440xxxx` FIRST.** *"The C's count of software semaphore
   writes to that page is **zero**, deliberately. Ours is unknown. If we have two writers, we are
   reproducing M5.38 and the spin is a symptom, not the disease."*
2. **Match the green vector** — `m2cefwd=0`, `m2hostsem=0`.
3. **Then build cont.34 FIX B** — one-shot `GPFIFO_SCHEDULE` of the **GR TSG** at doorbell time.
   ⊘ *"We already serve `GPFIFO_SCHEDULE` (#210) — this is about scheduling the GR TSG ourselves
   at doorbell time, which is a different thing."*
4. ⊘ **Do not port the M8.108 credit shortcut.**

## ★★ The headline that reframes the whole wall — the spin may be a CORRUPTION, not an absence

Same doc, §1. The `MC_SERVICE_INTERRUPTS` spin — **118 for the C, 175/76 for us** — was **not**
the guest waiting for something that never arrived. For the C it was the guest reading a
completion **already corrupted**: a lagging bridged host CE2 executed stale GPFIFO entries ~40 s
late and DMA-wrote payloads `1,2` **over the live value `0x1e`**; UVM's 32→64-bit wrap detector
(`uvm_gpu_semaphore.c:776`) read the backwards jump, bumped the software upper word, reconstructed
`completed_value 0x1_00000054 > queued 0x54`, and wedged the channel (`uvm_channel.c:205`).

**The fix (M5.38) was to DELETE THE SECOND WRITER**, not to build a completion plane.

⇒ This is a **competing explanation for our own frozen page** and it is not addressed by a sweep.
⚠ It is *compatible* with w274's measurement — a page sampled from ~70 s in is **frozen at 5 and
2**, and a frozen-after-corruption page and a frozen-after-nothing page look identical in a
snapshot. **The two are distinguishable only by watching the page from before the freeze.**

## ✔ Priority 1, done here (cheap, no boot) — and it comes back CLEAN

`grep` over `crates/**/*.rs` for writers to `0x2_0440xxxx` / `SET_REPORT_SEMAPHORE`: the only
hits are `crates/kayfabe-rt/src/completion_watch.rs`, which **decodes** the method
(`decode_report_semaphore`, `:137`) and asserts the VA `0x2_0440_fff0` (`:882`). It is a
**watcher, not a writer**. No CPU forge writes a payload into that page.

⇒ **On a source read, our software writer count at that page is ZERO — matching the C's green
configuration.** We do **not** appear to be reproducing M5.38's two-writer corruption.

⚠ **Scope this honestly:** that is a **grep over source**, not a census of writes on a live boot,
and this project's own rule is that **a census zero needs a known-positive**. The claim that
survives is *"no software writer exists in our tree"*, **not** *"nothing but the GPU wrote that
page on any boot"*. The strong form needs a boot-time write census with a known-positive.

★ This matters because it **removes** the doc's #1 candidate rather than confirming it — which
promotes items 2 and 3, and leaves the redirect's sweep still unranked by the source that
analysed the sequence.

## ⇒ Recommendation

**Do not start with the sweep.** The cheapest decisive next step is §6's item 3 —
one-shot `GPFIFO_SCHEDULE` of the **GR TSG** in our doorbell path — because the C's analysis
attributes `gp_get` advancing and *all 16 pool semas advancing* specifically to it.

⚠ **And check its currency before building**, because that is this project's most expensive
recurring failure and this doc is two days old: memory carries `§16.55 … OUTCOME P. The refusal
is NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`, and **w268 measured completions landing anyway** (*"all 8 GR
slots written by hardware, GET caught PUT in 32 ms, CUP2_RC=124"*). If `gp_get` already advances
on our bench, §4's step 3 has **already happened by another route** and its predicted effect has
**already been observed without fixing the wall** — which would retire it and genuinely promote
the sweep. **That check is a log read, not a boot, and it must happen before either is built.**

⊘ **A ruling's date is part of the citation** — including this one's.
