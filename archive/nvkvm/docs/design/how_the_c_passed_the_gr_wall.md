# How the C passed the wall we are standing on

**Status:** answered 2026-08-10, owner's question. The C reached `cuCtxCreate → 2048² matmul` at
`bad=0 maxerr=0` on a **stock** guest. We are stuck at a wall the C's own docs name. **How did it get
past?**

**Answer: (A) — a fix landed the NEXT DAY and the doc was never updated. And the fix was a THIRD
option that §0.7 never listed.**

> ### ✔ Verified by me before this was committed
> - `6b4a56b` (the §0.7 *"route B (real completion)"* ruling) — **2026-06-10**. ✔
> - `ceb13f5` — **2026-06-11**, *"mode2 M5.37-39: fix UVM tracking-sema wrap corruption + ghost-sweep
>   stall (**cuCtxCreate clears MC pump**)"*. ✔
> - `git merge-base --is-ancestor ceb13f5 264caa2` → **true**: the fix **is in the binary that
>   produced `cap3`**, the `bad=0` matmul capture. ✔
> - `ceb13f5`'s own body, verbatim: `0x1_00000001 -> 0x1_00000054 > queued 0x54 -> channel wedged ->
>   MC_SERVICE_INTERRUPTS pump spun forever (uvm_gpu_semaphore.c:776 / uvm_channel.c:205)`. ✔

---

## 1. ★★★★★ THE HEADLINE — it was never a MISSING completion. It was a CORRUPTED one.

The `MC_SERVICE_INTERRUPTS` spin — 118 occurrences for the C, 175 for us — was **not** the guest
waiting for something that never arrived. It was the guest reading a completion **we had already
corrupted**:

1. The completion-semaphore page was forward-mapped WB into the host VAS.
2. A lagging bridged host **CE2 executed stale GPFIFO entries 0/1 ~40 s late**, DMA-writing payloads
   `1,2` **over the live value `0x1e`**.
3. UVM's 32→64-bit wrap detector (`uvm_gpu_semaphore.c:776`) saw the backwards jump, bumped the
   software upper word, and reconstructed `completed_value 0x1_00000054 > queued 0x54`.
4. Channel wedged (`uvm_channel.c:205`) → **the pump spins forever.**

**The fix (M5.38) was: DELETE THE SECOND WRITER.** Gate the semaphore forward-map on `m2hostsem`
(`nvkvm_gpu_emul.c:5737-5751`; predicate `:5554`, `hostonly = m2exec && m2hostsem && is_user_ce`).
With `m2hostsem=0` the host GPU **cannot touch the live tracking-semaphore page**.

⇒ ★★★ **No completion plane was built. The hang was removed by removing a writer.**

⚠ **And this is a mechanism already banked in this project's own memory and never applied to our own
wall:** *"a backwards write is FATAL ON FIRST OCCURRENCE — UVM reads any decrease as a 2³² wrap,
exceeds `UVM_GPU_SEMAPHORE_MAX_JUMP`, and `UVM_ASSERT_MSG_RELEASE` is compiled into release builds."*
We had the mechanism. We did not connect it to the symptom.

---

## 2. REFUTED

- ⊘⊘ **My leading hypothesis — *"the C is default-accept, so it never produced the spin state"* — is
  REFUTED, and cleanly.** The acceptance policy is a **constant across the wall**: `0x20801702` had
  no table entry, fell to the *"echo with `NV_OK`"* default (`:3435-3436`, `post_status(…,0)` at
  `:3544`) — **and the C hung anyway** with that policy in force. Nothing about default-accept changed
  between the 118-occurrence hang (`a0abb69`) and the green matmul (`264caa2`). **Same handler, same
  `NV_OK`, opposite outcome.** ⇒ Neither necessary nor sufficient. Stop using it as the explanation.
- ⊘ **Shape (C), "the forge was sufficient"** — refuted **at our exact address**:
  `mode2_2nd_context_hang.md:1889-1891` — *"the pool is written by the **HOST GPU's** CE/GR
  `SET_SEMAPHORE`, **not by the emulator's parser** (zero SEMW to `0x2044xxxxx` all run), so it needs
  **REAL host execution**."*
- ⊘ **"The C took the M8.108 credit shortcut."** No such code exists — `service_zero`, `zero_budget`,
  `credit` all return nothing across `src/qemu/`.
- ⊘ **"Route B was built quietly."** Still false: `nvkvm_isolate_poll` has one caller, the Mode-1 path.

---

## 3. ★★ THE DISCRIMINATOR — unanimous across every committed capture

`ctrl` decoded at command-element offset 88 (`:2392`), plus a raw byte-scan for `02 17 80 20`:

| capture | records | `fn=76` controls | **`0x20801702`** | hermetic |
|---|---:|---:|---:|---|
| `cap1_coldboot_hermetic` | 359 062 | 80 | **0** | **yes** |
| `cap1b_coldboot_hermetic_d6` | 360 725 | 174 | **0** | **yes** |
| `cap2_stalequeue_negative` | 886 999 | 191 | **0** | no |
| `cap2b_stalequeue_nofn47` | 862 940 | 72 | **0** | no |
| **`cap3_matmul_forwarding`** | **532 824** | **191** | **0** | no |

**708 control elements across every committed C trace. Zero.** ★ Two of the five zeros are in
**hermetic** captures, so the `pci_dma_map` caveat does not apply to them: a full boot + `nvidia-smi -q`
with 174 controls and zero `0x20801702` establishes the control is **not routine in this guest** — it
appears only under the pathological spin. ⇒ `cap3`'s zero reads as *"the pump never started"*, not
*"the recorder missed it."*

⚠ **Honest caveat:** our **175** is on the **guest ioctl** plane (`nvdiff`); the C's **118** and this
**0** are on the **GSP RPC** plane. Zero-vs-118 is a **qualitative gap, not a validated ratio**. The
one artefact that would settle it: an `nvdiff` ioctl-plane capture of a **green C run**.

---

## 4. What the C did at OUR exact address

Our wall: `SET_REPORT_SEMAPHORE` → VA `0x2_0440fff0`, payload `1`, on a GR pushbuffer with **no launch
method** (#236). The C stood on the same page — `mode2_2nd_context_hang.md:1884-1891`: `cuCtxCreate_v2`
spins on **16 per-channel semaphores** in a sysmem pool at guest VA `0x20440ff00..0x20440fff0`,
16 slots × 0x10, `target=1`. ⇒ **`0x2_0440fff0` is slot 15 of that same pool, and payload `1` is that
same `target=1`.**

**What it did, in order:**
1. **Host-backed the pool page** — compute-aperture (`VA ≥ 0x2_0000_0000`) working-set sweep pins
   pushbuffers *and the pool* into the GR fvas via `back_and_map_sys`.
2. **Resolved the GR VAS PDB** — `chan_own_pdb` fallback, `populate_cvas` retry-until-resolve.
3. ★★★ **`GPFIFO_SCHEDULE`d the GR TSG** (cont.34 FIX B, *"the operative one"*) — one-shot per
   `(client, tsg)` in `nvkvm_m2_exec_doorbell` (`:4176-4194`, `:8038-8048`). Without it the 8 GR-family
   channels ring (`GP_PUT=1`) and the host never consumes (`gp_get` stuck 0). **With it, `gp_get`
   advances 0→4 and all 16 pool semas advance → `cuCtxCreate` returns.**

⇒ ★★ **"No launch method in the GR pushbuffer" is not a defect and not the blocker.** GR context-init
*is* methods plus a report-semaphore release. The C never synthesized a launch — it made the **real
host GR engine execute that pushbuffer**, and the engine wrote `0x2_0440fff0` itself.

★ **The C's forge for this method exists and was NOT what satisfied the poll**: `:6547-6570` parses
`0x1b0c`, writes on `RELEASE`, zeroes the FOUR_WORDS timestamp — and decodes `AWAKEN_ENABLE` **into a
log string only**. Per `:1890` it produced **zero** writes to this page. **Live, correctly aimed, and
irrelevant.**

---

## 5. The C's green configuration, and two divergences on our bench

From `cap3`'s self-describing header: `m2fwd=1 m2exec=1 **m2hostsem=0** m2cefwd=0 m2cexec=0
m2opaque=0 m2trace=0 m2romregs=0`, emulator md5 = `264caa2`.

⚠ **Divergence 1 — `m2cefwd` is ON on our bench and was OFF in the green.**
`bench_boot.sh:56` sets `NVKVM_M2CEFWD="${NVKVM_M2CEFWD:-1}"`. **We are not running the configuration
that went green.**

⚠ **Divergence 2 — a `:+` footgun on that line.** `run_mode2_vm.sh:113` expands
`${NVKVM_M2CEFWD:+,m2cefwd=on}` — so **`NVKVM_M2HOSTSEM=0` expands to `m2hostsem=on`**. *Any* non-empty
value, including `0` and `false`, **enables** the flag. If `m2hostsem` is ever on, M5.38's single-writer
rule is off and **the exact 118-occurrence corruption is re-armed.** ⇒ **Verify with the emulator's own
printed property line (`:9714-9721`), never with the env var.**

---

## 6. ⇒ What to do, in priority order

1. ★★★ **Audit our writer set at `0x2_0440xxxx` FIRST.** The C's count of *software* semaphore writes
   to that page is **zero**, deliberately. Ours is unknown. **If we have two writers, we are
   reproducing M5.38 and the spin is a symptom, not the disease.**
2. **Match the green vector** — `m2cefwd=0`, `m2hostsem=0`.
3. **Then build cont.34 FIX B, not route B**: one-shot `GPFIFO_SCHEDULE` of the **GR TSG** in the
   doorbell-exec path, keyed `(client, tsg)`. ⊘ We already *serve* `GPFIFO_SCHEDULE` (#210) — this is
   about **scheduling the GR TSG ourselves at doorbell time**, which is a different thing.
4. ⊘ **Do not port the M8.108 credit shortcut.** §0.7's verdict on it stands, untouched.

★ **The durable lesson:** a decision recorded in a doc outlived its own supersession by two months and
sent two bench lanes toward building something the C had already proved unnecessary. **A ruling's date
is part of the citation.**
