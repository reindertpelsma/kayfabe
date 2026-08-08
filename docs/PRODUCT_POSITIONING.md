# Product positioning — what kayfabe sells, and to whom

**Owner decisions, 2026-08-08.** This is a *decisions* doc, not analysis: each section states a
ruling and the reasoning that survives scrutiny. ⊘ Where a claim is an estimate rather than a
measurement it is marked `[unverified]` — several here are, and they should be checked before
anything is advertised on them.

## 1. The release commitment

> *"For 100% sure this project becomes public + advertised if it works what mode 1 C had + more
> mature."*

⇒ The release bar is **not matmul**. Matmul is Mode 2's proof of mechanism. The bar is the C's
**Mode 1** feature set reproduced on the Mode-2 architecture:

- CUDA at ~host parity (C measured GEMM 1.00×, LLM 0.97×, DMA 0.93× byte-exact; overhead was 100 %
  nesting, zero bare-metal)
- ★ **multi-process** — the rewrite's founding problem (`#14`), still open
- graphics / Vulkan, NVENC, 22 real GPU apps, 7B LLM at 63 tok/s, PyTorch
- plus **maturity**, scoped as the phase after the LLM app runs

**Target scope:** multi-tenant · **all major drivers** · **all Turing+** · parity perf · great
support.

⚠ `PRE_PUBLIC_CHECKLIST.md` currently holds **one** item (an NVENC note already re-baselined as
not-a-blocker). ⊘ It reads like a nearly-clear gate and is in fact an empty one. It needs building
out before it means anything.

## 2. ★★★ What we actually sell: HOSTILE-GUEST sharing, not GPU sharing

| | guest touches real hardware? | safe for an untrusted tenant? |
|---|---|---|
| **VFIO passthrough** | yes, fully | ⊘ **no** — a root guest can flash VBIOS/GSP firmware that **persists across VM teardown** onto the next tenant; PCIe config reach; incomplete **FLR** leaves memory residue |
| **Windows GPU-P style PV** | no, but a **trusted-guest** model | ⊘ **no** — documented as *not* a security boundary for hostile guests; ships for WSL2 and dev VMs, not multi-tenant hosting |
| **kayfabe** | ⊘ **never** — emulated GPU; intent forwarded as **unprivileged** host ioctls from a capability-dropped, namespaced isolate | ★ **designed for it** |

⇒ The firmware-flash attack is **structurally unavailable** here, not merely blocked: the guest
never reaches a register, and RM does not permit flashing from an unprivileged process.

★★ **Consequence for engineering priority.** The unprivileged isolate, the capability allowlist,
**refusing by name**, read-native/write-trap, the closed `/dev` `O_PATH` escape — ⊘ these are not
rigour for its own sake, **they are the product**. "Multi-tenant on consumer cards" without them is
passthrough with extra steps.

⚠ ⇒ **The adversarial security audit belongs ON the critical path**, alongside Mode-1 parity — not
in the maturity phase. When advertised, *"safe for hostile guests"* is **the claim being sold**, and
a sold claim needs an audit behind it, not design intent. Precedent: the C's `/dev` escape was found
by an **audit**, never by tests.

## 3. Competitive risks, ranked

**a. vGPU unlock maturing — LOW threat.** `[unverified]` It rides a **licensed** host driver and a
per-version patch, on a path that has grown harder across generations. Even at perfect reliability,
licence terms keep it out of commercial multi-tenant hosting. ⇒ It takes hobbyists, not customers.

**b. NVIDIA shipping Linux PV — the real one, but constrained.** They built it once (WSL2 GPU-P),
for a strategic partner, in a **single-tenant developer** shape. Doing it for Linux multi-tenant
cannibalises vGPU licensing and datacentre margin. ⇒ `[unverified]` medium-low probability, severe
impact. ★ **And if it follows the GPU-P model it is trusted-guest-only — which does not take this
market.** The version to actually watch is Linux PV **for datacentre parts only, licensed** —
extending the moat rather than opening it, and leaving consumer multi-tenant unserved.

**c. NVIDIA closing kayfabe specifically — LOWER than it first appears.** There are four places a
check could live and three do not work:

| location | why it fails as a defence |
|---|---|
| guest kernel driver | ⊘ open source — patchable |
| **GSP firmware** | ⊘ **we *are* the GSP** — a check there is a check we do not run |
| hardware root of trust | needs mandatory attestation; see below |
| ⚠ **libcuda (closed userspace)** | ← the actual soft spot |

★★ NVIDIA's own move to firmware offload — the thing that made this project possible — also removed
the natural place to hide a detection. **You cannot hide a check inside the component being
impersonated.**

⚠ On attestation: a challenge meaning *"prove you are a real NVIDIA GPU"* we can **forward to real
silicon**, which signs honestly — we intermediate a real GPU rather than emulate one. The version
that bites is attestation **bound to guest state** (Confidential Computing). `[unverified]`
mandatory CC on consumer parts looks commercially implausible — it would add a verifier dependency
to running a game offline and break every non-CC VM, container and CI runner.

⇒ **Hedge for all three: multi-version guest-driver support**, already on the roadmap for
portability. It doubles as defence.

## 4. ★ Provenance is a commercial asset, not just hygiene

Everything here derives from three clean sources: NVIDIA's **published** open kernel modules,
**black-box measurement** of a driver we legitimately possess, and public documentation.

⊘ **Do not accept leaked or disassembly-derived material, ever.** It would convert a defensible
product into an unshippable one, and it taints contributors permanently — which is exactly why
nouveau and Mesa developers refuse to look.

★ Note which knowledge travels: `traces/real_ga106/cuinit_ioctl_trace_real_ga106.txt` can ship with
the product, be cited in docs, and onboard a contributor in an afternoon. A private annotated binary
can do none of that however much it contains. **Measurement-derived knowledge is publishable;
disassembly-derived knowledge is not.**

⚠ And libcuda's closedness costs us little **by construction**: we meet it at the **ioctl boundary**,
which is observable with an `LD_PRELOAD` and no reverse engineering. §14.28–29 proved it — a
guest-side interposer found in one boot what sixteen fault injections could not. ⇒ Keep the trace
interposer a **maintained, first-class tool**: it is not only how we find walls, it is how we would
detect being closed out.

## 5. Why the moat helps us

NVIDIA's CUDA lock-in — the library stack, and kernels written in CUDA that cannot merely be
recompiled — means **the compute people want is specifically CUDA, on cards they can buy**.

⇒ A vendor-neutral virtualisation layer would be worth **less**, not more. Every year the moat
holds is a year this product's value rises. ★ And the same commercial logic that sustains the moat
argues **against** NVIDIA shipping consumer multi-tenant virtualisation themselves. ⊘ The two fears
in §3 do not coexist at full strength.

## 6. ⊘ Open question worth answering with data

**How much vGPU is deployed at one VM per GPU** — i.e. bought for **isolation and manageability**
rather than for slicing? `[unverified]` Whole-GPU vGPU profiles exist and are supported, and the
motives for choosing vGPU over passthrough at 1:1 are exactly ours: clean reset between tenants,
live migration, suspend/resume, manageability.

⇒ If that fraction is material it is a **validating** signal, not a threat: it proves enterprises
already pay real money for the property kayfabe provides. ⊘ We do not have the number. Checkable
from NVIDIA's published vGPU profile documentation and from what cloud providers actually offer.

## 7. Working posture (current)

★ **Discovery posture is sanctioned** until the LLM app runs: optimise for **walls falling and boots
run**, not test count. ⊘ Not licence to drop the boot-as-oracle, named refusals, or attributed
claims. Captures outrank tests in this phase. ⚠ The debt is real and the exit condition is explicit
— ⊘ do not let it outlive the LLM app.
