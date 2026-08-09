# Product positioning — what kayfabe sells, and to whom

**Owner decisions, 2026-08-08.** A *decisions* doc: each section states a ruling and the evidence
behind it. Every external claim carries a source URL or says plainly that it **has not been
verified**. ⚠ An earlier draft of this file asserted several of these without sources and one of
them turned out to be **wrong**; that correction is recorded in §3 rather than quietly fixed.

## 1. The release commitment

> *"For 100% sure this project becomes public + advertised if it works what mode 1 C had + more
> mature."*

⇒ The bar is **not matmul** — matmul is Mode 2's proof of mechanism. The bar is the C's **Mode 1**
feature set reproduced on the Mode-2 architecture: CUDA at ~host parity, ★ **multi-process** (the
rewrite's founding problem, still open), graphics/Vulkan, NVENC, 22 real GPU apps, 7B LLM at
63 tok/s, PyTorch — plus maturity, scoped as the phase after the LLM app runs.

**Target scope:** multi-tenant · all major drivers · **all Turing+** · parity perf · great support.

⚠ `PRE_PUBLIC_CHECKLIST.md` holds **one** item, already re-baselined as not-a-blocker. It reads
like a nearly-clear gate and is an empty one. It needs building out before it means anything.

## 2. ★★★ What we sell: HOSTILE-GUEST sharing, not GPU sharing

### The competing options are documented as trusted-tenant only — by their own vendors

**Microsoft, Discrete Device Assignment planning doc, "Security"** — verbatim:

> "Discrete Device Assignment passes the entire device into the VM. This pass means all
> capabilities of that device are accessible from the guest operating system. **Some capabilities,
> like firmware updating, might adversely affect the stability of the system.** … **You should only
> use Discrete Device Assignment where the tenants of the VMs are trusted.**"

<https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/plan/plan-for-deploying-devices-using-discrete-device-assignment>

★ Note precisely what it establishes: firmware update is **reachable from the guest**, and the
vendor's conclusion is **trusted tenants only**. ⊘ It does **not** say the effect persists to the
next tenant — see the honest limit below.

**Xen `SUPPORT.md`, x86/PCI Device Passthrough** — verbatim, verified against the raw file:

> "**Because of hardware limitations (affecting any operating system or hypervisor), it is
> generally not safe to use this feature to expose a physical device to completely untrusted
> guests.**"

<https://github.com/xen-project/xen/blob/master/SUPPORT.md>

⇒ ★★ *"affecting any operating system or hypervisor"* pre-empts the obvious rebuttal that a better
hypervisor fixes it.

### ⚠ The honest limit on the firmware argument

The **composite** claim — *"a root guest flashes GPU firmware and it persists to the next tenant"* —
**is not documented anywhere we found**. Each link is separately sourced; the chain is not. ⊘ Do
not state the composite as established. What is sourced: firmware update is guest-reachable (above);
NVIDIA's VBIOS signature lock has been broken publicly
(<https://github.com/notfromstatefarm/nvflashk>); and firmware-persistence attacks on bare-metal
cloud are documented in general (Eclypsium,
<https://eclypsium.com/blog/the-missing-security-primer-for-bare-metal-cloud-services/>).

### ★★ The stronger, fully-sourced claim: the scrub is the TENANT'S OWN DRIVER

⊘ Do not argue "FLR is broken". Argue this instead — it is provable from NVIDIA's published source:
scrub-on-free is a **driver flag**, default-on since GK110, disableable by registry key, and off
entirely on Windows non-TCC. Corroborated by peer review: Maurice et al., FC 2014, recovered data
across VMs under passthrough and found zeroing happens *"as a side effect of ECC and not for
security reasons"* (<https://s3.eurecom.fr/docs/fc14_maurice.pdf>).

⇒ **A cleanup performed by the departing tenant's own software is not a cleanup.** And the reset
path is demonstrably fragile in current hardware: the RTX 5090 / RTX PRO 6000 virtualization reset
bug forces a **host** reboot (<https://www.tomshardware.com/pc-components/gpus/rtx-5090-pro-6000-bug-forces-host-reboot>).

### Where kayfabe sits

| | guest touches real hardware? | vendor's own posture |
|---|---|---|
| **VFIO / DDA passthrough** | yes, fully | ⊘ "**only** … where the tenants of the VMs are trusted" (MS); "not safe … completely untrusted guests" (Xen) |
| **kayfabe** | ⊘ **never** — emulated GPU; intent forwarded as **unprivileged** ioctls from a capability-dropped, namespaced isolate | ★ built for untrusted tenants |

⇒ The guest never reaches a register, and RM does not permit firmware flashing from an unprivileged
process. ★★ **So the unprivileged isolate, the capability allowlist and refusing-by-name are the
product**, not rigour for its own sake.

⚠ ⇒ **The adversarial security audit belongs ON the critical path**, alongside Mode-1 parity. When
advertised, *"safe for hostile guests"* is the claim being **sold**. Precedent: the C's `/dev`
`O_PATH` escape was found by an **audit**, never by tests.

### ★★★ 2.1 Rootless on the host by construction — and that changes WHO can adopt it

**Owner ruling, 2026-08-09.** kayfabe requires **exactly the privileges an ordinary CUDA process
requires, which is none**. It uses the same `/dev/nvidia*` interface any `import torch` uses.

| | what the HOST ADMIN must do first |
|---|---|
| **NVIDIA vGPU** | install the vGPU Manager kernel module, stand up a **licence server**, configure mdev/SR-IOV — with licensing enforced **per VM** `[inferred]` |
| **VFIO passthrough** | bind the device to `vfio-pci`, set IOMMU kernel parameters, usually reserve hugepages `[inferred]` |
| **kayfabe** | ⊘ **nothing** |

⚠ The two comparison rows are `[inferred]` from NVIDIA's and the kernel's documented deployment
requirements; they are **not** measured here and carry no URL yet. ⊘ Do not upgrade them to
`[measured]` without one. Our own row is `[measured]` — see below.

★★ **The second-order consequence is the more important one: a TENANT can run it, not only an
operator.** vGPU and VFIO both require the *host administrator* to reconfigure the machine per
deployment; kayfabe requires nothing of the host at all. That changes **who can adopt it**, not
merely how easily — and it is the difference between a product an infrastructure team must bless
and one a user can start.

**Cite it as `[measured]`, because it is:**

- The isolate **surrenders every capability** and gets its own user namespace; an unprivileged,
  userns-confined process demonstrably drives the GPU (`2575177`).
- An **unprivileged process can obtain the PTIMER mapping** (`e20c3a2`).
  ⊘ ⚠ Not `977d5af` — that commit is *"#128: two prose inaccuracies I introduced, both counting
  things wrongly"* and has nothing to do with PTIMER. The wrong hash was caught only because it
  was checked before being written down (`a_wrong_citation_is_more_durable_than_none`).
- A static musl image in a **sealed memfd**, `execveat`'d into its own namespaces (`4cd39b0`).

#### ⊘ Three objections raised and deflated — recorded so they are not re-raised

1. *"We have never run non-root end to end."* **True, and it stays true until it is tested.** But
   it is one `sudo -u`: the C's Mode-1 VMM already made **every NVIDIA call unprivileged**, and
   the Rust port needs few ioctls beyond that set. ⇒ **Owed as an acceptance test AFTER `cup2`
   passes** — a confirmation, not a discovery.
2. *"The tap device needs `CAP_NET_ADMIN`."* ⊘ **This mistook a BENCH ARTIFACT for an
   architectural constraint.** `nvktap0` is how our *harness* wires the guest; rootless VMs use
   slirp / user-mode networking and need no privilege. ★★ **The harness's requirements are not
   the product's requirements** — worth stating as a general caution, because this is the second
   time a bench-local fact has been promoted to an architectural one (`CLAUDE.md`'s
   ssh-config trap, ⊘ corrected 2026-08-08).
3. *"`uffd` is `EPERM` unprivileged, so parity is unproven."* ⊘ **Checked and refuted.** No Rust
   file invokes `userfaultfd(2)` — `grep` for `SYS_userfaultfd`/`__NR_userfaultfd`/`UFFDIO_`
   over `crates/` returns **nothing**; every mention is a **doc comment describing a capability**.
   `docs/design/gl11_region_arguments.md:214-217` calls the region lock *"a capability we keep
   for a case we have not yet met, not a load-bearing part of the data plane"* — ⚠ **stated
   conditionally there** (*"if §2.1 survives the GSP build and §3 resolves to BAR2"*), so quote
   it with its condition or not at all — and records `[measured]` **0 cost unarmed**. ⇒ Unarmed
   and **not a live privilege requirement**; the syscall grep above is the load-bearing half of
   this bullet, the design quote only corroborates it. ★ Nor does the event plane reintroduce
   one: an **eventfd is a plain fd** and
   **`/dev/kvm` is a group** on essentially every distribution.
   ⚠ Correction to the framing as given: the claim *"uffd appears in only two files"* is wrong —
   it appears in ten or more, all prose. The **substance** (no live syscall) survives; the count
   does not.

#### ★ The honest wording to publish TODAY

> *"No privilege is required for anything we have built."*

⊘ **Not** *"full parity with root"*. That earns its **"full"** once the remaining planes land —
the event/interrupt switchboard (#199) above all.

⚠ **And record the acceptance test here, because this is exactly the kind of claim that is true
when written and quietly false a month later:** a **rootless end-to-end boot** — VMM, isolate and
guest all as an ordinary user with no capabilities — owed after `cup2` passes. Until that boot
exists, §2.1 is an argument from component measurements, not from a running system.

## 3. ⊘ A correction: what Windows GPU-P actually says

An earlier draft claimed *"Microsoft documents GPU-P as a trusted-guest feature, not a security
boundary."* ⊘ **That is not supported and must not be repeated.**

**What IS documented** (`well-sourced`): the GPU-PV path — Windows Sandbox, Application Guard,
WSL2, confirmed by Microsoft to be the same mechanism at
<https://devblogs.microsoft.com/directx/directx-heart-linux/> — carries the verbatim warning
*"enabling virtualized GPU can potentially increase the attack surface of the sandbox"* on two
independent Learn pages, with a policy to disable it. Its hardened **"Secure VMs"** mode is
**opt-in** and works by *removing* capability — driver escape calls banned, IOMMU isolation
mandated — which tells you what the default permits
(<https://learn.microsoft.com/en-us/windows-hardware/drivers/display/gpu-paravirtualization>).
And the predecessor feature, RemoteFX vGPU, was **disabled and removed** in 2020 because the
vulnerabilities were *"architectural in nature"* (KB4570006).

⚠ **What contradicts the old claim:** Windows Server 2025 GPU partitioning — **also called GPU-P** —
is documented as SR-IOV-based and *"provides a **hardware-backed security boundary** … prevents
unauthorized access by other VMs."*
<https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/gpu-partitioning>

⇒ **The name covers two different implementations.** Say the old sentence unqualified and a reader
produces that page.

★★★ **The corrected argument is stronger, not weaker.** Microsoft's secure GPU-P requires
**SR-IOV plus a licensed vGPU driver** — i.e. datacentre hardware. ⇒ The secure version does not
reach consumer cards, and the version that does reach them is the one carrying an
attack-surface warning. **That is our gap, and it is now sourced rather than asserted.**

## 4. Competitive risks

**a. vGPU unlock — LOW threat, and now sourced.** It covers **Maxwell 2.0 / Pascal / Turing only** —
the project states *"THIS MEANS THAT YOUR RTX 30XX or 40XX WILL NOT WORK"*. ⇒ The entire
Ampere/Ada/Blackwell consumer line has no unlock, which is precisely the market we target.

**b. NVIDIA shipping Linux PV — the real risk.** No such announcement found. The disincentive is
commercial: it cannibalises vGPU licensing. ⚠ This probability judgement **has not been verified**
and rests on incentives rather than observation. ★ The version to watch is Linux PV for
**datacentre parts only, licensed** — extending the moat rather than opening it.

**c. NVIDIA closing kayfabe — lower than it first appears.** Four places a check could live, three
of which fail: the guest kernel driver is **open source** (patchable); **GSP firmware — we *are* the
GSP**, so a check there is one we do not run; a hardware root of trust needs attestation; ⚠ leaving
**libcuda** as the real soft spot. ★★ NVIDIA's own move to firmware offload — what made this project
possible — removed the natural hiding place. **You cannot hide a check inside the component being
impersonated.**

⚠ On attestation: **has not been measured**; reasoning from published mechanism. A challenge meaning
*"prove you are a real NVIDIA GPU"* can be **forwarded to real silicon**, which signs honestly — we
intermediate a real GPU rather than emulate one. The version that bites is attestation **bound to
guest state** (Confidential Computing), which is **opt-in and Hopper/Blackwell-only today, with
nothing on consumer parts**.

⇒ **Hedge for all three: multi-version guest-driver support**, already on the roadmap for
portability.

## 5. Does anyone pay for isolation rather than slicing?

**Whole-GPU vGPU profiles exist and are supported** — `A40-48C`, 48 GB, **max vGPUs per GPU = 1**
(<https://docs.nvidia.com/ai-enterprise/release-8/latest/infra-software/vgpu/reference/ampere.html>).
★ And NVIDIA's own feature matrix pushes compute customers there: unified virtual memory is
*"limited to … profiles that allocate the entire frame buffer"*
(<https://docs.nvidia.com/vgpu/knowledge-base/latest/vgpu-features.html>).

⚠ **But the motive is documented as manageability, not isolation.** Every vendor reason found for
vGPU-over-passthrough is live migration, suspend/resume, DRS and monitoring — NVIDIA's vGPU FAQ and
VMware. ⊘ **No vendor sells vGPU on tenant isolation or clean reset.** The isolation argument
appears in vendor documentation only *negatively*, about passthrough (§2). Those are two separate
facts and must not be merged into one.

⊘ **The split between whole-GPU and sliced deployments: not found.** Searched NVIDIA, VMware and
analyst listings; nothing segments deployments by profile size. ⚠ A "30 % of new VDI deployments"
figure surfaced untraceably in a search summary and is **deliberately not used**.

## 6. ★ Provenance is a commercial asset

Everything derives from three clean sources: NVIDIA's **published** open kernel modules (vendored as
`ogkm-580.159.04`, cited throughout as `ogkm-580: file:line`), **black-box measurement** of a driver
we legitimately possess (the committed captures under `traces/real_ga106/`), and public
documentation.

⊘ **Never accept leaked or disassembly-derived material.** It taints contributors permanently —
which is why nouveau and Mesa developers refuse to look — and would convert a defensible product
into an unshippable one.

★ Note which knowledge travels: `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt` can ship,
be cited, and onboard a contributor in an afternoon. A private annotated binary can do none of that.
⚠ And libcuda's closedness costs little **by construction** — we meet it at the **ioctl boundary**,
observable with an `LD_PRELOAD` and no reverse engineering (§14.28–29: a guest-side interposer found
in one boot what sixteen fault injections could not). ⇒ Keep the interposer a **maintained,
first-class tool**: it is how we would detect being closed out.

## 7. Working posture

★ **Discovery posture is sanctioned** until the LLM app runs: optimise for **walls falling and boots
run**, not test count. ⊘ Not licence to drop the boot-as-oracle, named refusals, or attributed
claims. Captures outrank tests in this phase. ⚠ The debt is acknowledged and the exit condition is
explicit — ⊘ do not let it outlive the LLM app.
