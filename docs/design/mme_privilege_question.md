# Can guest-authored MME microcode carry privilege?

**Answer: NO.** ⇒ **Nothing to refuse, and a constraint I asserted is retired.**

**Status:** answered 2026-08-10, read-only source audit. Version: `research_clones/ogkm` = **610.43.02**,
load-bearing citations re-checked against **580.159.04** and identical in substance. ⚠ `ogkm` is
versioned, not the spec.

> ### ✔ Verified independently before committing
> - **Zero privileged methods in the graphics/compute classes** — `grep -c PHYS` / `grep -ci priv` over
>   `clc697.h` (`AMPERE_A`), `clc6c0.h` (`AMPERE_COMPUTE_A`), `clc7c0.h` (`AMPERE_COMPUTE_B`):
>   **`PHYS=0 PRIV=0` in all three.** ✔
> - **NVIDIA's own containment model** — `ogkm/src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:241-243`,
>   verbatim: *"Guest-RM clients can allocate a privileged channel to perform actions such as updating
>   page tables in physical mode or scrubbing. **Security for these channels is enforced by VMMU and
>   IOMMU**"*. ✔

---

## 1. ⊘ THE CLAIM THAT IS RETIRED — mine

I asserted, in `gr_execution_boundary.md` and to the owner:

> *"The pushbuffer carries 39 dwords of guest-authored MME microcode, and the MME's output is methods
> ⇒ **no method-level allowlist can be sound**, therefore **the VA space must become a containment
> surface** before the GR route can open."*

**The premise is true. The conclusion does not follow.** MME output re-enters the graphics method
pipeline of **the same channel at the same privilege level**, so it is a **subset of what the guest
could already write as literal pushbuffer dwords**. Nothing in the method stream can change what the
channel is allowed to do.

★★★ **The owner's reframe, which is the correct one:**

> *"Any guest VA space is untrusted — I would distrust any VAS for the VMM anyway. So you shouldn't
> put any data the guest may reach in a VAS it can point at **in the first place**. And guest
> userspace channels are **passthrough** to unprivileged isolated channels on the host, so all
> commands an unprivileged context is allowed to make are allowed — we don't need to inspect. This is
> the same for `nvproxy`, Mode 1 and Mode 2."*

⇒ **The boundary was never syntactic.** It is (a) the **channel's privilege**, fixed at allocation,
and (b) **address-translation hardware**. Inspection was never load-bearing, so its impossibility
costs nothing.

---

## 2. Why the capability does not exist

**Graphics/compute classes carry no privileged method.** Census above. Every memory operand in
`AMPERE_A` / `AMPERE_COMPUTE_A/B` is a **GPU VA**. The classes are explicitly user-allocatable —
`RS_FLAGS_ALLOC_NON_PRIVILEGED` (`resource_list.h:2122-2132`).

**The MME's own methods are ordinary class state.** `LOAD_MME_INSTRUCTION_RAM{,_POINTER}`,
`LOAD_MME_START_ADDRESS_RAM`, `SET_MME_SHADOW_RAM_CONTROL` (`clc697.h:55-72`) sit between
`WAIT_FOR_IDLE` and the rest of the class. `SET_MME_MEM_ADDRESS_A/B` is a **40-bit GPU VA** with **no
`TARGET`/`APERTURE` field anywhere in the file** (`:681-728`) ⇒ **MME DMA is GMMU-translated in the
channel's own VAS by construction.** The operand *cannot* name a physical address.

**Three near-misses, all disposed of:**

| candidate | why it does not apply |
|---|---|
| **CE physical mode** — `SET_SRC/DST_PHYS_MODE` (`clc6b5.h:56-73`) genuinely bypasses the GMMU | different **engine and class**, unreachable from MME; and gated at alloc by `NVOS04_FLAGS_CHANNEL_DENY_PHYSICAL_MODE_CE` (`alloc_channel.h:158-170`) |
| **Host `MEM_OP` TLB invalidate** — names a PDB by **physical address + aperture**, `PDB_ALL` reaches every context (`clc56f.h:141,175-186`) | a **host** method the guest can already emit literally; it **invalidates**, never reads or writes; MME cannot reach the host method space. ⚠ Worth its own look if a *host-method* question is ever opened |
| **Software methods** — `GP100_UVM_SW` `CLEAR_FAULTED` / `FAULT_CANCEL` | the object is `RS_FLAGS_ALLOC_PRIVILEGED` (`resource_list.h:1638-1645`) — the gate is the **ioctl** |
| **`SET_FALCON00..31`** (`clc697.h:4140-4234`) | 32 opaque data slots, `_V 31:0` only — no address, aperture or target; RM never emits one. Ctx-switched class state, not a door |

---

## 3. ⚠ Hardware-enforced or RM convention? — the honest answer

**No register bit was found**, and this is stated plainly because reading a comment as a bit is a
failure mode already recorded in this tree.

- **Hardware, found:** a **per-PTE privilege bit** in the GMMU — `NV_MMU_PTE_PRIVILEGE` /
  `NV_MMU_VER3_PTE_PCF_PRIVILEGE_*` (`swref/published/rubin/gr100/dev_mmu.h`), surfaced by RM as
  `NV0080_CTRL_DMA_PTE_INFO_PARAMS_FLAGS_PRIVILEGED`. ⚠ `dev_mmu.h` is vendored only for
  `rubin/gr100` here, not `ampere/ga10x`.
- **Not found:** the PBDMA/instance-block bit that makes a *channel* privileged. The vendored
  `dev_pbdma.h` files are stripped; on Turing+ the open module is GSP-only, so instance-block
  programming lives in unvendored GSP firmware. `privilegeLevel` is computed in
  `kernel_channel.c:235-283` and **shipped to physical RM**.
- **Explicitly convention, not enforcement:** `uvm_hal_ampere_host_method_is_valid` returns `true`
  immediately unless SR-IOV heavy (`uvm_ampere_host.c:32-38`); `uvm_channel_is_privileged` is
  **unconditionally `true`** on bare metal (`uvm_channel.c:4003-4009`). ⇒ The whole
  `method_is_valid` family is a **UVM self-check for the vGPU plugin path, not a security boundary.**
  **Read it as a comment, not a bit.**

**Net:** privilege is a property of the **channel object**, established through the **ioctl** path and
enforced by **address-translation hardware**. It is not a property of the method stream, and nothing
in the method stream can change it.

---

## 4. ★ Independent confirmation — `nvproxy` does not inspect methods

`grep -rni "pushbuffer|method stream"` over `gvisor/pkg/sentry/devices/nvproxy/` → **zero hits**;
`grep -rn MME` → zero. From `gvisor/g3doc/user_guide/gpu.md`:

> *"gVisor doesn't introduce any additional hardware-level isolation beyond that which is configured
> by the NVIDIA kernel-mode driver. **There is no validation of things like DMA buffers. The only
> checks are done in seccomp-bpf rules to ensure `ioctl(2)` calls are made on supported and
> allowlisted `ioctl`s.**"*

It constrains `AMPERE_A` only at **allocation**, behind `nvconf.CapGraphics`
(`nvproxy/version.go:456`). ⇒ **The canonical reference deliberately places the boundary at the
ioctl and lets the GPU MMU do containment** — identically for Mode 1 and Mode 2.

**And the C artifact agrees:** it *parses* pushbuffers to **observe** completions
(`nvkvm_gpu_emul.c:6048-6055,6163-6200`) with **no rejection path** — an unrecognised method is
logged, never blocked. Its actual allowlists are all at the **ioctl/object layer**
(`nvkvm_ctrl_allowlist.h`, `nvkvm_fe_alloc_allowlist.h`, …). **It ran LLMs and PyTorch on that basis.**

---

## 5. ⊘ DO NOT NAME THE LIMITATION

The owner proposed declaring *"kayfabe does not support privileged-op macro extenders"* — with the
constraint that **the capability must exist before the limitation can be named.** It does not.

⇒ **Naming it would create a SEVENTH VACUOUS GATE** in this tree: a refusal that reads as a security
property in the docs and enforces nothing, because it can never fire. The security audit already
found six. ★ **The owner's own ordering constraint is what prevents this, and it is the right rule:
establish existence, then refuse.**

⊘ **The Mode-1 cost question is therefore moot** — there is no refusal to pay for. (Had the answer
been yes, the oracle would have been Mode 1's app set — CUDA, multi-process, graphics/Vulkan, NVENC —
because **public release is committed at Mode-1 parity**, making it release-gating rather than
academic.)

---

## 6. ★ What survives — a claim about TECHNIQUE, not containment

**Static pushbuffer scanning cannot predict the method stream the engine will issue.** MME microcode
computes method addresses and data at runtime from MME data RAM and `MME_DMA_READ`
(`clc697.h:687-694`), so a scanner reading dwords cannot enumerate what actually runs. `w227c`
measured 39 dwords of guest-authored microcode on 8/8 channels, so the guest really does use this on
the live GR path.

⇒ **That kills a technique nobody needs.** Every method MME can emit is one the guest could have
written literally, on a channel whose privilege it cannot change. **It does not license the
constraint I derived from it.**

---

## 7. ⇒ What this changes

- **`gr_execution_boundary.md`'s containment clause is retired.** The FB crossing is still required —
  **for the engine to FUNCTION** (three of four bound operands must resolve), **not for containment.**
  ★ That moves the bar from *"prove nothing else is reachable"* to *"prove these three resolve"*,
  which the existing address census already measures.
- **The invariant to state instead**, and it is stronger because it binds *us* rather than the guest:
  > **VMM state is never placed where a guest VA can name it.**
  It holds even in an address space we do not control, and it does not depend on the guest's behaviour.
- ⊘ **S1 is not gated on containment.** Its remaining preconditions are (a) our state not co-located
  with guest-addressable VA space, and (b) the operands resolving.
