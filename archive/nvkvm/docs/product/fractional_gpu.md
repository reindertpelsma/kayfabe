# Product direction: fractional / quota'd GPUs via nvkvm

Two distinct offerings fall out of nvkvm's design. Both exploit that nvkvm
**brokers every allocation** through QEMU — a policy insertion point NVIDIA's
stack doesn't expose.

## 1. License-free vGPU on MIG hardware
- MIG (A100/H100) gives a hardware-isolated GPU slice. Pin QEMU to a MIG
  instance, run a KVM guest on it, forward via nvkvm.
- Result: vGPU-grade isolation (own VM + own kernel) **without NVIDIA's vGPU
  license** — which is the expensive gate today.
- Audience: datacenter operators with MIG-capable GPUs.
- Isolation: hardware (MIG) → safe for adversarial multi-tenant.

## 2. Software-quota'd fractions on COMMODITY (consumer) GPUs  ← the novel one
- Consumer RTX cards have no MIG, and NVIDIA deliberately offers no per-process
  VRAM quota. But nvkvm already intercepts `RM_ALLOC_MEMORY`, so a **per-VM VRAM
  cap** is a tiny feature: count bytes per isolate in QEMU, reject past the cap.
- Result: "sell 1/4 of a 4090" to N inference containers, each a KVM guest,
  fractioning enforced by *our* policy layer — on hardware NVIDIA won't fraction.
- Directly attacks real waste: a 4090 running a 1.5B model is ~90% idle.
- The accounting plumbing is the same insertion point as GET_PID_INFO
  ([[get_pid_info_findings]]).

### Honest caveats
- **Memory quota is easy; compute QoS is harder.** Capping VRAM is trivial; fair
  compute sharing needs channel-submission throttling or NVIDIA's time-slicer —
  real work, not free.
- **No hardware partition on consumer = cooperative, not hostile, tenancy.**
  Shared SMs/caches → noisy-neighbor + side-channel exposure. Fine for "my own
  containers / cheap inference"; use MIG+nvkvm when tenants are adversarial.

## Framing
MIG+nvkvm = isolation-grade vGPU without the license.
nvkvm-quota on consumer = density/cost play NVIDIA structurally won't offer.
The brokering layer as a policy point (quota / accounting / QoS) is itself moat,
beyond raw isolation.

## Smallest next demo
Per-VM VRAM cap: a byte counter per isolate incremented at RM_ALLOC_MEMORY in
QEMU, configurable ceiling, reject with NV_ERR_INSUFFICIENT_RESOURCES past it.
Show two guests on one consumer GPU, each capped, one OOMing at its limit while
the other runs unaffected.
