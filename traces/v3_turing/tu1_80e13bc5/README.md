# Turing, first contact — TU116 (GTX 1660 SUPER) at `80e13bc5`

**STATUS: LIVE, 2026-09-28.** kayfabe v3's first run on any Turing die. Box: vast 53080587, GTX 1660
SUPER (`0x21C4`, VBIOS 90.16.4F.40.3D), Xeon W-2133 host, nested KVM, host driver 580.159.04 open
(`driver.log`), provisioned from `claude/kayfabe-gpu-testing-m0cv1q` at `80e13bc5` (`prov.log`, READY).
Run order: bare metal first (CLAUDE.md), then gates, thin guest, CUDA ladder (`/root/tu_run.sh`).

| step | result | file |
|---|---|---|
| bare-metal raw client, 30 arms, 120 s | **30/30** (90 s total) — the client needs no Turing change | `bare.run`, `tubare_*.log` |
| v3 gates | **7/9** — gate3 and gate4: `Ring(Rewrite { gp: 0, why: ForeignClass { subch: 4, class: 50613 } })`, 50613 = `0xC5B5` TURING_DMA_COPY_A. A **harness** bug: the gates' hand-written CE list started at Ampere; the product's predicate is derived (fixed in `46a66099`, hardware re-check pending) | `gates.log` |
| thin guest, 30 arms, 180 s | **0/30, CRASH ×30** in 1–2 s, empty serial: kf3 **realize refused** — `host facts: 3 host fact(s) refused by name: gr_info: reply not servable: Unservable { cmd: 0x20801228, why: "a GR info entry RM's own readers require non-zero is zero" }` (+ `memory_system`, `gr_static` depending on it) | `fast_tuthin_rpc-mixed-allocs_qemu.log`, `tuthin_suite.out` |
| CUDA ladder, host (bare metal) | cup2 PASS; cup3 / cup8 / cup8bench **FAIL on bare metal** (empty grade) — not a kayfabe result; the ladder's host programs on Turing need a look | `ladder_host.log` |
| CUDA ladder, fat guest | 0/4 (`boot_rc=2`: the same realize refusal) | `ladder_guest.log` |
