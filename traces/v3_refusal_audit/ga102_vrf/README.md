# Refusal audit evidence — GA102 (RTX 3090), host and guest `580.159.04`, 2026-09-26

**STATUS: LIVE.** The captures behind `docs/design/V3_REFUSAL_AUDIT.md`, taken on vast 52742480
(`vrf`) with `scripts/bench/refusal_audit/` and the nvdiff `LD_PRELOAD` recorder
(`archive/nvkvm/tests/mode2/nvdiff/nvdiff_shim.c`). Every `.jsonl.zst` is one workload's
complete ioctl stream (params before and after every `/dev/nvidia*` ioctl); the `.log` beside it
is the workload's own output.

| dir | what | kf3 revision |
|---|---|---|
| `host/` | bare metal, same box, same binaries (`h2`; `deviceQuery` from `h3`) | — |
| `guest_before/` | the kf3 fat guest before the fixes (`g2`; `deviceQuery` from `g3`) | `131f4841` (`g3`: `08c14423`, which already served the sysmembar and unset but still refused cudart's clock pair) |
| `guest_after/` | the 23 workloads after the fixes (`g4`) | `f8e6a513` |
| `guest_rebased/` | 8 of them again at the branch's rebased tip (`g5`) | `393012fd` |
| `summary/` | `*.res` verdict rows; userspace status matrices and kernel-ledger deltas before/after (`.txt` and `.json`); the verification logs; gates, fast suite and CUDA ladder outputs | as named |

⚠ `131f4841`, `08c14423` and `f8e6a513` are pre-rebase commits of the `v3-refusals` series and
are on no branch now; `393012fd` is the citable revision.

Re-derive any table in the doc from here, no GPU needed:

```sh
mkdir -p /tmp/m && for f in host/*.jsonl.zst; do w=$(basename $f .jsonl.zst); \
  [ -f guest_after/$w.jsonl.zst ] || continue; mkdir -p /tmp/m/$w; \
  ln -sf $PWD/$f /tmp/m/$w/host_r1.jsonl.zst; ln -sf $PWD/guest_after/$w.jsonl.zst /tmp/m/$w/guest_r1.jsonl.zst; done
python3 scripts/nvdiff_status.py matrix /tmp/m          # needs an ogkm tree for names (--ogkm)
```
