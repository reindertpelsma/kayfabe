# Windows performance status, 2026-10-17 (evidence summary for the top-level README)

**STATUS: RESEARCH, 2026-10-17.** A summary of runs that live on branches, not on master. Every number says where it came from.
M = measured (a run on a named revision), I = inferred. Heaven = Unigine Heaven 4.0, Ultra, 1080p, Windows 11 guest, stock guest driver 580.88,
host RTX 4070 (AD104), host driver 595.91.07. Heaven's own score for the run window (260 s), not a screenshot.

## 1. Production path vs the experimental scan-off builds (M)
| arm | revision | extra flags | D3D11 fps | OpenGL fps |
|---|---|---|---|---|
| production path (scan on), controls | 28a0ee9c / b4040417 | none | 89.1, 98.7 (the 0a746117 base: 99.8 to 101.7) | 220.5 (base: 231.8 to 233.2) |
| experimental: scan off after the first object binding (5080) | 134dcb54 | `KF3_SCAN_OFF_AFTER_5080=2` | 196.2, 193.1, 202.2 | 358.5 (b4040417: 352.7) |
| same, private ring 2,048 / 32,768 entries | 134dcb54 | `KF3_PRIV_RING_ENTRIES` | 194.5 / 196.7, 196.1, 197.2 | not run |
| same, plus lazy private-USERD reads | 134dcb54 | `KF3_LAZY_OBSERVE=16` | **299.1, 296.5, 299.2** | not run |
| native Windows, VFIO | earlier campaign | n/a | 330 (not re-measured) | 342 (not re-measured) |

Run table (fps, power, SM and memory utilisation per run) and the exact flags per boot are in the gap agent's records
(branch `claude/d3d11-gap-20261016`, `scripts/bench/d3d11_gap/`, run labels L1a..c, L4a, L64a..c, Z16a..c, PFc, PFm2, G0a).
Power at the lazy-observe arm: about 157 W (the 512-entry arms: about 125 W; native, earlier campaign: about 191 W).

## 2. What these numbers are not
- The scan-off and lazy-observe builds are research code (branches `claude/perf-lifecycle-measure-20261016`, `claude/d3d11-gap-20261016`). They are not on master and are
  not mergeable. They skip reading the guest's pushbuffers after the channel's first object binding; if a guest does something this does not expect, the failure is explicit
  (a reset of the guest's own channel, a driver reset in the guest), not a silent wrong result. That claim is by design and by the one registration-window experiment, not
  yet by an app matrix.
- The safe implementation of the same idea (the "lifecycle": scan until the first binding, then pass pointers, with a scanned window around migrations) is on branches
  `claude/win-lifecycle-p1b-20261016` and `-p1c-` with GPU-free tests and two independent reviews, and has NO Windows hardware run yet.
- Which half of the lazy-observe change carries the gain is not yet known (the GP_PUT cache or the less frequent private-USERD reads); the split runs were still queued. The
  mechanism (the relay's per-turn slow loads) is inferred from code and from the controlled arms, not proven.
- Only Heaven was run with these builds. No application matrix (Minecraft, Edge, GravityMark, Blender) exists for them. Minimum fps in these runs is lower than average
  (about 43 to 45 in the lazy-observe arm).
- Native 330/342 fps are inherited from an earlier campaign on the same GPU. A re-measurement on a rented box (branch `claude/native-vast-20261016`) was CPU-bound
  (about 133 fps on both APIs, GPU 35 to 40% busy) and cannot replace them.
- Single host, single GPU, a few runs per arm; run-to-run spread was 1 to 10% in earlier campaigns.
