# v3-ioeventfd evidence (2026-09-30) — ALL NESTED

Box: vast instance 53510558, RTX 3060 (GA106), AMD EPYC 7K62 host (machine 30524), a KVM guest
itself (nested), Linux 6.8.0-59-generic, host driver 580.159.04 (open). Design and reading:
`docs/design/V3_DOORBELL_IOEVENTFD.md`.

| dir | revision | what |
|---|---|---|
| `mcio1_43293417/` | `43293417` | `merge_check.sh` — crate tests 1700/0, gates 9/9, KF3_RC=0, bare metal 30/30, FG_RC=0, thin suite 30/30 (fast path OFF = the property's default). `mcio1.log` is the verdict; `merge_check_detail.tar.gz` holds the stage logs |
| `dbl1_43293417/` | `43293417` | `dbfast_lane.sh dbl1` — thin suite **ON 30/30** (823 passthrough doorbells, all by eventfd, 0 trapped; CeUtils Translated doorbells handed by the drainer); guest-timed doorbell stores ON/OFF (`DBL_EXIT`); CUDA ladder OFF 4/4 and ON 4/4 with `cup8bench` per-launch rows; GPU-free tests + bench A/B on the box. `dbl1.log` is the summary; the tarballs hold the suite/ladder/probe outputs and sample QEMU logs |
| `mcio2_ca7a5006/` | `ca7a5006` | ★ **the merge bar at the final code revision** (every code change of the branch; later commits are docs/evidence/harness): crate tests **1701/0**, gates **9/9**, **KF3_RC=0**, bare metal **30/30**, FG_RC=0, thin suite **30/30** (fast path OFF = default) |
| `dbl2_ca7a5006/` | `ca7a5006` | `dbfast_lane.sh dbl2 suite_on,launch` at the final code revision — thin suite **ON 30/30**; `cup8bench` alone ×3 per mode (OFF / ON / ON + `KF3_DBFAST_SPIN_US=100`) |
| `mgb1_b351af9d_gb206/` | `b351af9d` (code = `ca7a5006`) | ★ **Blackwell** vast 53522821 — RTX 5060 Ti (GB206, 10de:2d04), EPYC 7K62, nested: merge bar crate tests **1701/0**, gates **9/9**, **KF3_RC=0**, bare metal **30/30**, thin suite **30/30** (OFF) — the first GB206 run of kayfabe |
| `llm1_ca7a5006/` | `ca7a5006` | LLM decode (Qwen2-0.5B eager, `llm_parity` guest_pm lane) — host lane (`llm1_prov_host.out`) and the guest boots ON / OFF / spin / ON / OFF (`*.lp`: the `LP` lines incl. `LLM_DOORBELLS`, `LLM_KVM_EXITS`, `LLM_THREAD_CPU`; `*.dbfast`: the device's fast-path status) |
