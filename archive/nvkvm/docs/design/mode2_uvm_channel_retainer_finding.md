# `UVM_CHANNEL_RETAINER` (0xc574) — what the C artifact actually did

**Date:** 2026-08-10 · **Scope:** research artifact (`/workspace/nvidia-gpu-passthrough`) +
`research_clones/ogkm-580.159.04`. Findings only, no code changes. Written to answer a
`kayfabe`-side blocker: `GSP_RM_ALLOC hClass=0xc574` refused by the Rust, 16 failures in the
failing stage, last kernel event before ~175 s of silence.

---

## 0. LEAD: what this refutes

### ⊘ REFUTED — the leading hypothesis ("the retainer is a fallback; serve the dup instead")

The hypothesis was: *`UVM_CHANNEL_RETAINER` is UVM's fallback for when channel duping is not
supported, so if we served the dup UVM actually attempts, UVM would never allocate a retainer at
all.*

**There is no dup to serve.** In 580.159.04 `nvGpuOpsRetainChannel` has exactly one path to
retain a channel, and it is the retainer:

- `nv_gpu_ops.c:10213-10231` — the code goes `hChannelParent = …` → the "Channel duping is not
  supported" `NV_PRINTF` → `pRmApi->Alloc(…, UVM_CHANNEL_RETAINER, …)`, **straight-line, no
  conditional**. (This corroborates the already-established fact that `:10221` is unconditional;
  it also shows there is no `#else` arm elsewhere in the function.)
- The only dup in the whole function is of the **TSG**, not the channel:
  `nvGpuOpsGetChannelTsgInfo` at `nv_gpu_ops.c:9917-9979` produces `hDupTsg`, and that dup runs
  **before** the retainer (`:10199`) and only supplies the retainer's `hChannelParent`
  (`:10213-10214`). Serving it does not remove the retainer — it is a *prerequisite* of it.
- ⇒ "Channel duping is not supported" is a statement about **this driver build**, not about our
  emulation. There is no branch we can satisfy. The retainer is the only path, and the fix is
  necessarily *"answer the retainer"*.

### ⊘ REFUTED — "the C never hit this" (I expected a negative; it is not one)

The C **did** hit `0xc574`, in a Mode-2 run, and it is recorded:
`docs/design/mode2_execfwd_keystone_plan.md:227`.

### ⊘ REFUTED — "route B (real host completion) is what the C does"

`docs/design/mode2_cuctxcreate_resume.md:296-309` **decides** route B (poll a real host eventfd,
deliver via GSP `POST_EVENT`) and explicitly rejects the M8.108 credit shortcut. **It was never
built for Mode 2.** `nvkvm_isolate_poll` / `nvkvm_isolate_unpoll`
(`src/qemu/nvkvm_isolate.c:1991`, `:2013`) have exactly one caller each —
`src/qemu/nvkvm_isolate_handlers.c:2068`, `:2080` — both taking `VirtIONvgpu *nv`, i.e. **Mode 1**.
`nvkvm_gpu_emul.c` never calls them. This is a **plan item quoted as design**; see §5.

---

## 1. Q1 — did the C ever hit `0xc574`? **YES, once, recorded.**

One hit in the entire artifact outside `research_clones/` and `gvisor/`:

> `docs/design/mode2_execfwd_keystone_plan.md:227`
> `- The status=0x33 SHADOW allocs in THIS run are classes 0xc574 + 0x0079 (NV01_EVENT_OS_EVENT),`

"SHADOW allocs" = `nvkvm_m2_shadow_fwd` (`src/qemu/nvkvm_gpu_emul.c:6749`), which forwards **only
what the guest sent** (`:6752-6754`: `if (fn != 103 && fn != 10) return;`). So the guest's UVM
**did** issue `GSP_RM_ALLOC hClass=0xc574`, the C forwarded it to the real host GPU, and the host
returned `0x33 NV_ERR_INVALID_OBJECT_HANDLE`.

⚠ **Gating check:** `m2fwd` is `DEFINE_PROP_BOOL(..., true)` at `nvkvm_gpu_emul.c:9928`
("always on"), and `trace` is `true` at `:9924`, so both the forward and the `DIAG ALLOC` decode
(`:2355-2367`) ran in the green configuration. This is not a dark path.

## 2. Q2 — what did the C do with it? **Answered `NV_OK` unconditionally, and ignored the host's refusal.**

Two independent halves:

**(a) To the guest: unconditional `NV_OK`, for every class.**
The C's fake GSP has **no refusal vocabulary at the RPC level and no alloc allowlist on the
Mode-2 path**:

- Response is a verbatim echo of the request element: `nvkvm_gpu_emul.c:2952-2954`
  (`static uint8_t resp[…]; memset(…); memcpy(resp, cmd, 4096);`).
- It is posted at `:3544` — `nvkvm_m3_post_status(s, resp, fn, 0 /* rpc_result NV_OK */)`.
- `nvkvm_m3_post_status` writes that value into both result words:
  `:1753` `rpc.rpc_result`, `:1754` `rpc.rpc_result_private`.
- **All three** call sites of `nvkvm_m3_post_status` pass `0`: `:1790` (`GSP_INIT_DONE`),
  `:1820` (`POST_EVENT`), `:3544` (every serviced RPC). There is no path that posts a non-zero
  `rpc_result`.
- The only per-class special-casing for `fn == 103` is the M8.4 GR-object `paramsSize` rewrite
  (`:2967-3037`) — a *size* fix, not a status. `0xc574` matches none of it.

That is **sufficient for the guest**, which is why it worked:

- `rpcRmApiAlloc_GSP` (`ogkm .../vgpu/rpc.c:11232-11249`) does
  `status = _issueRpcAndWait(...)`; on `NV_OK` it deserializes and returns success. The body's
  own `rpc_gsp_rm_alloc_v03_00.status` field (`g_rpc-structures.h:1491-1502`, at element offset
  `+96`) is read **only on the error path** (`rpc.c:11248`).
- `_issueRpcAndWait` (`rpc.c:1821`) returns `NV_OK` iff
  `vgpu_rpc_message_header_v->rpc_result == SUCCESS` (`rpc.c:1994`, `:2010`).
- ⇒ **`rpc_result = 0` alone makes the retainer alloc succeed.** The echoed body's stale `status`
  word is never consulted.

**(b) The alloc has no output the guest reads.**
`NV_UVM_CHANNEL_RETAINER_ALLOC_PARAMS` is `{ NvHandle hClient; NvHandle hChannel; }`
(`class/clc574.h:37-41`) — **both fields are inputs**. `uvmchanrtnrConstruct_IMPL`
(`gpu/fifo/uvm_channel_retainer.c:41-101`) writes nothing back into the params; its entire effect
is host-internal refcounting (`kfifoChidMgrRetainChid`, `memdescAddRef` on the channel instance
block). ⇒ **An empty `NV_OK` echo is not a lie here — it is observationally complete.** Under
Mode 2 the emulator *is* the chId/instmem owner, so there is nothing to refcount.

⚠ Note this is the *inverse* of the `an_in_annotation_is_not_a_transport_fact` trap: there the
`[IN]`-marked body still had to be echoed because RM copied it back. Here both fields are `[IN]`
**and** the constructor writes nothing, so echoing the request bytes is both necessary and
sufficient. The C's blanket `memcpy(resp, cmd, 4096)` satisfies both cases by construction.

**(c) The host shadow forward failed and nothing checked.**
`nvkvm_m2_shadow_fwd`'s header comment (`:6741-6748`) states the contract: *"replay the guest's RM
alloc on the real host GPU in PARALLEL — the guest still proceeds on the faked GSP response, so
this is non-disruptive."* The function returns `void`; `nvkvm_isolate_ioctl`'s status is captured
into locals and, on this path, discarded. The `0x33` in the progress log was found by **reading
QEMU's log**, not by any check in the code.

⇒ **`0xc574` is unforwardable-in-practice and the C's architecture made that a no-op.**

## 3. Q3 — why our path differs

Not because of duping. Because of **default polarity**:

| | C artifact (Mode 2) | kayfabe (Rust) |
|---|---|---|
| `GSP_RM_ALLOC`, unknown class | echo + `rpc_result = NV_OK` (`:2953`, `:3544`) | refused |
| `GSP_RM_CONTROL`, untabled cmd | echo + `body.status = NV_OK` (`:3057`, `:3435-3436`) | refused |
| host shadow-forward failure | logged, discarded (`:6749`, `void`) | — |

The C is **default-accept**; the Rust is **default-deny**. `0xc574` was never a rung for the C
because it never *could* be: the C had no mechanism by which any class could fail.

This is the same shape as the already-recorded limit *"the diff can never be green end-to-end
because the C has no refusal vocabulary"* — but sharper: it is not only that the C cannot *refuse*,
it is that **for `0xc574` the C's inability to refuse is accidentally correct**, because the class
has no observable output and its effect is host-internal bookkeeping the emulator already owns.

⇒ **Serving `0xc574` as a bookkeeping-only `Ok` with the request body echoed is the C-verified
behaviour**, and it is what carried `cup8` to `bad=0 maxerr=0`. There is no evidence for any
richer treatment, because the C never produced any.

★ Caveat on the strength of this evidence: this is a **behavioural** result (a green end-to-end
run with the class answered `NV_OK`), not a capture of a real GA106's reply. It is exactly the
kind of claim the oracle's fifth limit warns about in the other direction — but note the direction
differs: here we are not decoding an empty capture as a value, we are observing that the class has
**no reply value to decode** (`clc574.h:37-41`, all-`[IN]`, constructor writes nothing).

## 4. Q4 — the C's progress meter, and whether this rung is named

The C had **two** ladders, both committed:

1. **`docs/MILESTONES.md`** — the outcome ladder (cup8 `bad=0 maxerr=0` at `:11`; llama.cpp;
   PyTorch; then `#12` multi-process L1/L2/L3a fixed, L3b open at `:26-45`).
2. **`docs/design/mode2_cuctxcreate_resume.md`** — the numbered **rung** ladder (`§0.1 … §0.7`),
   and **`docs/design/mode2_execfwd_keystone_plan.md`** — the `M5.x`/`M8.x` PROGRESS LOG 1-7 with
   the same numbering the code comments cite.

**`UVM_CHANNEL_RETAINER` / `0xc574` is NOT named in either ladder.** It appears exactly once in
the repo, as an incidental line item inside PROGRESS LOG 5 (`mode2_execfwd_keystone_plan.md:227`),
listed among *"the `status=0x33` SHADOW allocs in THIS run"* — an observation, never a rung.

**But the rung the Rust is standing on IS named — under the other symptom.** The
`175 × MC_SERVICE_INTERRUPTS` half of the blocker is the C's `§0.6`/`§0.7`:

- `mode2_cuctxcreate_resume.md:265` — *"0.6 MILESTONE (2026-06-10): cuCtxCreate CRASH FIXED +
  VERIFIED (M8.4), next = MC_SERVICE_INTERRUPTS hang"*.
- `:283-284` — *"cuCtxCreate now HANGS in the `MC_SERVICE_INTERRUPTS` (0x20801702) poll loop —
  QEMU echoes `NV_OK+zeros` and the guest polls forever (118 occurrences)."*
  (Our Rust measures 175. Same wall, same shape.)
- `:292` `§0.7` — the decision, and **what came next**:
  *"Decision: route B (real completion), NOT the M8.108 credit-shortcut. The shortcut fakes the
  completion without running the work = the oracle's dead end (green poll, no matmul)."*
  and *"**Keystone reduces to: engage execution-forward for the GR channel.**"* with the exact
  missing link named at `:305-307`: `nvkvm_m2_exec_doorbell` (M5.9) fires 0× for GR because
  `nvkvm_m2_populate_cvas` bails — `chan_own_pdb` returns 0 for GR client `0xc1d00003` — and the
  M5.30 `SET_PAGE_DIRECTORY` capture is the PDB source to wire in.

⇒ The C's own ladder says the `MC_SERVICE_INTERRUPTS` spin is **not** a control-plane refusal
problem. It is *"nothing ever completed"*. That is the same diagnosis as
`the_plane_rings_but_does_not_complete`.

## 5. ⚠ What the C BUILT vs what it PLANNED (the §0.7 trap)

`§0.7` reads like design. It is a **decision that was not implemented in Mode 2**:

| §0.7 says | what is in the tree |
|---|---|
| "reuse the POLL half (arm host eventfds, `ppoll`, `ISOLATE_RESP_POLL_EVENT`)" | `nvkvm_isolate_poll` (`nvkvm_isolate.c:1991`) is called **only** from `nvkvm_isolate_handlers.c:2068` (Mode-1 `VirtIONvgpu`). `nvkvm_gpu_emul.c` never calls it. |
| "the DELIVERY hop is the emulated GSP `POST_EVENT`" | ✔ built — `nvkvm_m3_post_event` (`:1806`), `nvkvm_gsp_deliver_events` (`:1849`) |
| completion is driven by a **real host** signal | ✘ — `nvkvm_gsp_deliver_events` has **one** caller, `:4365`, inside the **doorbell-write handler**, fired on `if (any_completed)` from the emulator's own GPFIFO walk. |

⇒ The C's `MC_SERVICE_INTERRUPTS` poll terminates because the emulator, having decided a channel
completed, **forges** `POST_EVENT` + SWGEN0 (`nvkvm_gsp_raise_swgen0`, gated one-batch-outstanding
at `:1855-1863`). Route B was never wired. This is the "the C forges completions, so a green diff
says nothing about the completion plane" limit, located in source.

Also note there is **no handler at all** for `0x20801702` in the C: it is absent from
`mode2_initctrl_ga106.h`'s `nvkvm_ctrl_resps[]` (`:6207`), so it falls through to the
*"else: void/SET control — echo with status=NV_OK"* default (`nvkvm_gpu_emul.c:3435-3436`). Its
appearance in `nvkvm_ctrl_allowlist.h:124` is the **Mode-1** allowlist (consulted by
`nvkvm_ctrl_cmd_allowed(uint32_t)` in `nvkvm_isolate_handlers.c:601`, called on the virtio path)
and has no effect in Mode 2.

⚠ **Gated path, flagged per instruction:** the one piece of `0x20801702` logic in Mode 2 —
the `m2_poll_kick` completion-retry (`nvkvm_gpu_emul.c:3040-3055`) — is gated on
`nvkvm_m2_multiproc(s)`, which is armed only when a **second** dup-source compute client appears
(`:2812-2822`). In every single-process green run (`cup2`, `cup8`, `cup8_iter`) **it never ran**.

## 6. Scope limits of this write-up

- Everything here is **static source evidence** plus the C's own committed logs. No bench, no
  boot, no capture was run for it.
- The `0x33` datum is a **single** progress-log line from 2026-06-10 (PROGRESS LOG 5), from a run
  whose source revision is not recorded in the doc. It establishes *that the class was seen and
  forwarded*; it does not establish a count or a per-run frequency.
- The reference traces under `traces/mode2_c_reference/` were **not** searched for `0xc574`
  (they are zstd record streams; a decode pass would be needed). If a stronger claim than
  "answered `NV_OK` by construction" is ever wanted, `cap3_matmul_forwarding` is where the
  `fn=103 hClass=0xc574` records would live — but §2(a) makes the answer structural, not
  statistical: **there is no code path by which the C could have answered anything else.**

## 7. Actionable, for the Rust side

1. **Do not chase a dup.** There is none (§0).
2. **Serve `0xc574` as bookkeeping-only success**, echoing the request body. Justified by
   `clc574.h:37-41` (both fields `[IN]`) + `uvm_channel_retainer.c:41-101` (no writeback), not
   merely by "the C did it".
3. **Do not forward it to the host.** It is a kernel-internal-client class over a channel handle
   that does not exist in the isolate's client; the C measured `0x33` (§1).
4. **`GPFIFO_GET_WORK_SUBMIT_TOKEN` is the statement the `goto error` skips** — serving the
   retainer only *reaches* it. The C reached it and returned the **host** channel's token
   (`nvkvm_gpu_emul.c:8029`, `:9037`, `:9587`, all `0xc36f0108` on the host channel that
   `shadow_fwd` created with the same `hObject`, per `:9029`).
5. **Expect `MC_SERVICE_INTERRUPTS` to remain after the retainer is served.** Per the C's own
   §0.6→§0.7, it is a completion-plane symptom, not a control-plane one — and the C's answer to
   it was a **forgery** (§5), which the C's own §0.7 text calls *"the oracle's dead end (green
   poll, no matmul)"*.
