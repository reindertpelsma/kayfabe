# v3-ioeventfd evidence (2026-09-30) — ALL NESTED

Box: vast instance 53510558, RTX 3060 (GA106), AMD EPYC 7K62 host (machine 30524), a KVM guest
itself (nested), Linux 6.8.0-59-generic, host driver 580.159.04 (open). Design and reading:
`docs/design/V3_DOORBELL_IOEVENTFD.md`.

| dir | revision | what |
|---|---|---|
| `mcio1_43293417/` | `43293417` | `merge_check.sh` — crate tests 1700/0, gates 9/9, KF3_RC=0, bare metal 30/30, FG_RC=0, thin suite 30/30 (fast path OFF = the property's default). `mcio1.log` is the verdict; `merge_check_detail.tar.gz` holds the stage logs |
