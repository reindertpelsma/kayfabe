# V3 FAMILY PORT — Blackwell (GB203, RTX 5080): the first FSP-booted family on hardware

**STATUS: IN PROGRESS, 2026-09-26 (branch `v3-blackwell`).** Filled in as measured; §0 is the
running verdict.

## 0. Verdict (running)

- Bare metal, raw client: see §5.
- Thin guest: see §5.

## 1. Box

vast `52730218`, RTX 5080 16 GB (`0x2C02`, **GB203**, `MC_GET_ARCH_INFO` arch `0x1B0` = GB200
consumer), VBIOS `98.03.3B.40.17`, Intel i9-14900 (the vast KVM image is itself a VM: the thin guest
is a **nested** KVM guest). Host driver swapped closed 575.51.03 → **580.159.04 open**
(`provision_host_driver.sh`, `OPEN_MODULE=yes`); guest driver the same `.run`, `-m=kernel-open`.
