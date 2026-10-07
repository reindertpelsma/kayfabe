# V3 NON-GSP CONTEXT — what a non-GSP guest does with the golden context, and what we would serve

**STATUS: DESIGN-ONLY, 2026-09-25 (branch `v3-promote`).** Research; nothing here is built. The
GSP-guest half — `GPU_PROMOTE_CTX` / `GPU_EVICT_CTX` satisfied by the twin — IS built on the same
branch (`kf_rm::chanlink`, `kf_qemu::chan::CtxBind`); this doc answers the owner's follow-up: *"We
also need to support non-GSP, so worth looking at whether non-GSP handles the actual content and
what we need to stub inside it (check nova or nouveau)."*

Sources (read-only, no clone made): Linux at `research_clones/linux` (commit `6f3ed7fec`),
`drivers/gpu/drm/nouveau` (abbreviated `N/` = `…/nouveau/nvkm/`) and `drivers/gpu/nova-core`;
ogkm 580.159.04 at `research_clones/ogkm-580.159.04` (`O/`). Citations are file:line in those trees.

## 1. Verdict

1. **The golden-context CONTENT does not matter to us, on either path.** A non-GSP guest (nouveau)
   reads the golden image back once and copies it verbatim into every channel's context, patching
   at most nine header words; no CPU code consumes those bytes afterwards, and under kayfabe no
   hardware ever loads a guest context (the host twin runs on host RM's own context). An all-zero
   image cannot change execution.
2. **The generation PROTOCOL is small to fake** — a FECS mailbox state machine answering seven
   methods, plus PGRAPH as an always-idle write sink. ~1–2 weeks.
3. **What is NOT small is everything around it.** FECS/GPCCS only run after ACR secure boot via the
   SEC2 RTOS (2–4 weeks to emulate), and a non-GSP guest issues **no RPCs at all**: channels,
   runlists, instance blocks, page tables and TLB invalidates arrive as raw MMIO/PRAMIN traffic.
   Recovering intent from that is a **second front end comparable to the whole GSP path** —
   months, not weeks. Golden context is a rounding error inside it.
4. **Which guest is "non-GSP" at all:** ogkm has **no** monolithic path for dGPUs (§5); nova-core
   is GSP-only and has no GR (§6). The realistic non-GSP guest is **nouveau booted with
   `NvGspRm=0`** (its default is GSP on Turing/Ampere, `N/subdev/gsp/tu102.c:403`), or the closed
   proprietary driver in monolithic mode, whose golden path is not inspectable from source.

## 2. How nouveau generates the golden context

Lazily, on the **first GR channel**: `gf100_gr_chan_new` calls `gf100_grctx_generate` while
`gr->data == NULL` (`N/engine/gr/gf100.c:441-450`). One function for Fermi → GA10x
(`N/engine/gr/ctxgf100.c:1436`); tu102/ga102 only supply tables and hooks (`ctxtu102.c:48-75`,
`ctxga102.c:52-77`).

**At engine init** (`gf100_gr_init_ctxctl_ext`, `gf100.c:1749`): FECS/GPCCS are loaded through ACR
(`nvkm_acr_bootstrap_falcons`, `:1770-1774`); `0x409800`/`0x41a10c`/`0x40910c` are zeroed, both
falcons started, `0x409800` bit0 polled (`:1784-1797`); then the FECS methods below.

**The FECS mailbox protocol** (every method): clear `0x409800`, data → `0x409500`, method →
`0x409504`, poll `0x409800` (≤ 2 s). Ctxsw-control methods report in `0x409804` (1 OK / 2 error,
`:751-768`).

| method | meaning | nouveau reads back | cite |
|---|---|---|---|
| `0x21` | set watchdog (`0x7fffffff`) | nothing | `gf100.c:973-980` |
| `0x10` | discover MAIN image size | `0x409800` ≠ 0 **is the size** → `gr->size` | `:957-970` |
| `0x16` | discover zcull image size | stored, never used | `:941-954` |
| `0x25` | discover PM image size | stored, never used | `:925-938` |
| `0x03` | bind pointer (`0x80000000 | inst>>12`) | bits `0x10` OK / `0x20` err | `:834-850` |
| `0x09` | WFI golden save | bit1 OK / bit2 err | `:815-831` |
| `0x38`/`0x39`/`0x04` | stop/start ctxsw, halt pipeline | `0x409804` | `:771-812`, `:2331` |

(`0x30`–`0x32` ELPG reglist methods exist but are dead code, `if (0)`, `:1822`.)

**Generation** (`ctxgf100.c:1436-1558`): FE power force-on (`0x404170`), FECS reset (`0x409614`;
ga102 also `0x41a614`, `ga102.c:61-74`); allocate `0x80000 + gr->size`, point the instance block's
engine context at it (`inst+0x210/0x214 = VA+0x80000 | 4`, `:1487-1490`); bind (`0x03`); CPU-write
header words `+0x1c = 1`, `+0x20/+0x28/+0x2c = 0` (`:1494-1503`); then `gf100_grctx_generate_main`
(`:1342-1432`) — **all PGRAPH register traffic**: `sw_ctx` as plain MMIO (`gf100.c:1079-1093`),
bundles through ICMD (`0x400200/0x400204/0x40020c/0x400208`, polling `0x400700` bit2, full idle on
`GO_IDLE`, `gf100.c:1096-1133`), `sw_method_init` via `0x40448c/0x404488` (`:1136-1157`), idle polls
on `0x400700`, `0x200` bit12, `0x40060c` (`:1050-1071`); WFI golden save (`0x09`); then
**read-back of the whole image** into kernel memory: `gr->data[i/4] = nvkm_ro32(data, 0x80000 + i)`
(`ctxgf100.c:1538-1545`).

Netlist firmware: gm200–tu10x `sw_nonctx`/`sw_ctx`/`sw_bundle_init`/`sw_method_init`
(+`sw_veid_bundle_init`) (`gk20a.c:294-300`, `gm200.c:222-243`, `tu102.c:136-203`); GA10x one
`NET_img.bin` with region ids (`ga102.c:271-323`). `sw_nonctx` is applied at init, not per context
(`gf100.c:2351-2362`).

## 3. What the CPU does with the content

- **Per channel** (`gf100_gr_chan_bind`, `gf100.c:320-352`, verified): allocate `gr->size`, copy
  `gr->data` verbatim, patch `0xf4/0xf8 = 0`, `0x10` = patch-list count, `0x14/0x18` = patch-list VA,
  `0x1c = 1`, `0x20/0x28/0x2c = 0` (non-firmware path: `0x00`/`0x04`).
- The "mmio list" is the per-channel **patch buffer** of `(addr, data)` pairs FECS applies at first
  load (`gf100.c:452-486`, `ctxgf100.c:995-1004`); it carries the VAs of the global pagepool /
  bundle CB / attrib CB / RTV CB (`ctxgf100.c:1022-1029`, `ctxgv100.c:62-112`, `ctxtu102.c:39-46`).
- **Nothing reads a channel context back.** Later GR access is register-only (`0x409b00`,
  `0x409808`/`0x40981c`, `gf100.c:745-748`, `:983-997`, `:1614`). No zcull/PM pointer is ever set.
- Instance-block binding: engine ctx at `inst+0x210/0x214`, valid bit `inst+0x0ac` bit16
  (`N/engine/fifo/gv100.c:90-104`); subcontexts: VEID0 only (`N/subdev/mmu/vmmgv100.c:31-59`).

⇒ **The content is opaque to the guest after the one read-back, and meaningless to us.** The only
value that matters is `gr->size` (it sizes every per-channel allocation): answer `0x10` with the
host's real main-context size or larger.

## 4. What our emulated device would have to serve a non-GSP guest

| piece | fakeable? | what we serve | size |
|---|---|---|---|
| FECS mailbox state machine (`0x409500/0x409504/0x409800/0x409804`) | **yes** | `0x10` → host main-ctx size; `0x16`/`0x25` → any non-zero; `0x03`/`0x09`/`0x21`/`0x38`/`0x39`/`0x04` → immediate OK. ⊘ Never execute a golden save: the image stays as the guest wrote it | small |
| PGRAPH (`sw_ctx`, ICMD bundles, method init, idle polls) | **yes** | an absorbing write sink; every idle/status register reads idle | small |
| Instance block `inst+0x210` | **yes** | bookkeeping only (which guest context a channel names) — the twin's context is host RM's | small |
| FECS/GPCCS falcon start (`CPUCTL`, `0x409800` bit0 ready) | yes | falcon register model | small |
| **ACR + SEC2 RTOS** (HS boot, WPR regs, msgqueue `NV_SEC2_ACR_CMD_BOOTSTRAP_FALCON`, `N/engine/sec2/ga102.c:99-118`, `N/subdev/acr/base.c:105-168`; TU: `ucode_ahesasc`/`asb`, `N/subdev/acr/tu102.c:95-155`; GA10x `lazy_bootstrap`, `N/subdev/acr/ga102.c:150-183`) | yes, but it is a protocol | pretend RTOS: accept the HS image unverified (we are the chip), post init, ack bootstraps | **2–4 weeks** |
| **Intent recovery without RPCs** — RAMFC (`N/engine/fifo/gv100.c:41-61`), runlist submits, PDB, page tables, MMU invalidates, doorbells | no shortcut | a second front end: every statement the GSP path gets as an RPC must be rebuilt from hardware-level state | **months** |

⚠ Constraint check: none of the fakeable rows needs a CPU data plane, a forwarded privileged verb
or a guest-visible VMM address. The intent-recovery row is where the standing rules would be
tested hardest (MMU invalidate as a trap, PRAMIN traffic), and it is out of scope here.

## 5. ogkm has no non-GSP dGPU path (so the owner's "non-GSP" cannot mean ogkm)

- README: the open modules "must be used with GSP firmware" (`O/README.md:18`).
- `rm_set_rm_firmware_requested` forces `request_firmware = TRUE`,
  `allow_fallback_to_monolithic_rm = FALSE` (`O/src/nvidia/arch/nvalloc/unix/src/osapi.c:4930-4937`,
  verified), on every probe (`O/kernel-open/nvidia/nv-pci.c:1622`); a missing GSP image is fatal
  (`osinit.c:1826-1831`); pre-Turing is refused (`osapi.c:3565-3606`, `gpu_mgr.c:1063-1066`).
- The physical golden path (FECS method interface) is **not in the tree** — `grep` for
  `0x409500`/`0x409504`/`0x409800`/golden save finds nothing. Kernel RM only creates the golden
  channel as a GSP client (`kernel_graphics.c:465-508`, `:2136`) and promotes (§ the GSP half).

## 6. nova-core

GSP-only and has no GR at all: FWSEC-FRTS → SEC2 booter → GSP boot → `SetSystemInfo`/`SetRegistry`
→ init-done → static info (`nova-core/gsp/boot.rs:50-86`, `:141-235`;
`gsp/commands.rs:168-237`). Its only golden/FECS mentions are bindgen constants
(`gsp/fw/r570_144/bindings.rs:166`, `PROMOTE_CTX = 111`). ⇒ A nova guest is a GSP guest; the built
promote handling already covers it at the RPC level.

## 7. For comparison — the GSP half (built on this branch)

A GSP-client guest's CPU-RM owns its context buffers (`bClientRmAllocatedCtxBuffer`,
`O/src/nvidia/src/kernel/gpu/gpu_registry.c:153-156`) and asks physical RM (us) to initialize /
promote them (`kernel_graphics_object.c:51-159`; UVM bind `nv_gpu_ops.c:10854-10904`). It reads
**nothing** back from those buffers (the only GR buffer CPU-RM reads is the FECS event buffer, which
it fills with `0xde` itself, `fecs_event_list.c:1545-1547`). So on BOTH paths the conclusion is the
same: the guest's context bytes are write-only from its point of view, and the host twin's own
context is what the hardware runs.

## 8. Open questions (owner)

1. Is non-GSP meant to be nouveau-`NvGspRm=0` (inspectable, §2–4) or the closed monolithic driver
   (needs a trace; no source)? The sizing above is for nouveau.
2. If pursued, the golden-context fake is not the first milestone — ACR/SEC2 and the RPC-less
   channel front end are. Rank accordingly.
