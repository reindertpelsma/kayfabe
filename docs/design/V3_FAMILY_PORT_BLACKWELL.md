# V3 FAMILY PORT — Blackwell (GB203, RTX 5080): the first FSP-booted family on hardware

**STATUS: IN PROGRESS, 2026-09-26 (branch `v3-blackwell`, rebased on master `283a5304`).** Filled
in as measured; §0 is the running verdict. Every result names the revision it was measured at.

## 0. Verdict (running)

- **Bare metal, raw client: 30/30** at `92b9f912` (pre-rebase; the client fixes of §3), 25/30 before them.
- **Thin guest:** see §5.

## 1. Box

vast `52730218`, RTX 5080 16 GB (`0x2C02`, **GB203**, `MC_GET_ARCH_INFO` arch `0x1B0` = the consumer
Blackwell architecture), VBIOS `98.03.3B.40.17`, Intel i9-14900. The vast KVM image is itself a VM,
so the thin guest is a **nested** KVM guest. Host driver swapped closed 575.51.03 → **580.159.04
open** (`provision_host_driver.sh`: `OPEN_MODULE=yes`); the guest runs the same `.run` with
`-m=kernel-open`. The provisioning scripts already pick the open flavour on both sides.
