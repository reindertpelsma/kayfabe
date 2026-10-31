# Resume point: Claude coordinator, 2026-10-13

**STATUS: LIVE, 2026-10-13 ~11:55 CEST.** Written so a new session (Claude after the weekly reset, or any agent) can continue from the repo alone. Models: Sonnet 5.5 by default; Opus 5.5 only if Sonnet is insufficient (owner, 2026-10-13).

## Where the work is
- **Codex's branch** `origin/codex/windows-matrix-20261011` (tip `ec45aad99` at 11:17): a default-off Windows USER ring relay (private USERD + private GPFIFO, worker mediation) that makes GravityMark Vulkan PASS (run 1630, 181 s, flag `KF3_WIN_USER_RING_RELAY=1`).
  `ec45aad99` makes that route the default for Windows user channels. Its resume doc: `docs/RESUME_CODEX_WINDOWS_20261011.md` (on that branch). Codex stopped (capacity/safety-classifier errors); this session continues its validation.
- **Design docs** (this line, `origin/integration/windows-20261010`): `design/V3_WINDOWS_USER_RING_RELAY.md` (section 9 = coordinator revision), `design/V3_CHANNEL_RC_SCOPE_20261012.md` (branch `claude/rc-scope-20261012`: channel RC scope is UNKNOWN from source; two-VM tests M1-M3 pending),
  `design/V3_CONSTANT_STRATEGY.md` + `V3_CONSTANT_LISTS_20261011.md` (per-die constants: tiers, 140-row inventory, merged lists; reverse-engineering list empty for now), `design/V3_PROVENANCE.md` (draft, lawyer review pending), `OWNER_RULINGS.md` §AF-§AH.
- **Flag-free Windows validation of `ec45aad99`:** Linux gates 9/9 + gate 10, USER birth 12/12, fast 30/30 and the desktop smoke PASSED (Codex, 11:17-11:22). Windows run 1650 (Codex) failed only in the harness (default `max_tdr=3` counted three `nvlddmkm 153` events as TDRs, the restart path then broke the IOMMU hook).
  Run 1660 (this session, `--max-tdr 99`, script `/data/claude-default-windows-ec45aad99.sh` on the host, copy in `claude/host-tmp-scripts-20261013`): `edge_video_h264` PASS, `vectorAdd_demo` PASS, `gravitymark_d3d12` rc 0 with image and 33921 timing lines and zero TDR/WER/GSP cycles but harness verdict FAIL `no-gpu-proof`
  (the GPU-engine proof reads `3d`/`copy` counters and not the compute engine; INFERRED a proof-check gap, check frame times against GPU speed), `gravitymark_vk` was still running. Evidence: host `/var/lib/kf-windows-20261005/claude-vulkan-default-windows-ec45aad99/` (`EXIT`, `relay/summary.md`, `relay/apps/*`), log `/data/claude-default-windows-ec45aad99.log`.
  Earlier (`72741e2b`, 2026-10-11) D3D12 had 3 guest TDRs and 1 GSP cycle: the TDRs are gone.

## Next steps
1. Record run 1660's final result under `traces/` on a Claude branch (do not push to Codex's branch) and push; no default-shipping claim until the flag-free Vulkan oracle passes.
2. D3D12: verify the harness proof gap (compute engine counter) and the GPU-speed frame times; if confirmed, fix the proof check.
3. Run the app matrix on the default route (79 earlier passes at risk of regression from from-birth mediation), measure submit-to-completion latency and CPU cost on Edge video, D3D12 and the desktop.
4. Two-VM containment test (RC scope doc M1-M3) before trusting that an unscanned trigger only hurts its own guest; shadow mode and the refusal-class root-causing are in the coordinator message in `design/V3_WINDOWS_USER_RING_RELAY.md` section 9.
5. Add `0x20808165` (NVENC refusal) to `design/v3_constant_inventory.tsv`; host probe for the 17 layout-only rows; the raw-GSP spike (`V3_CONSTANT_STRATEGY.md` section 4).
6. Merge Codex's branch into integration only after review (security: hostile guest, vCPU non-blocking), exact-revision Linux + Windows hardware tests, CI.

## Host state and rules
the trusted test host (RTX 4070, host driver 595.91.07, Windows 580.88). Take the GPU lock with `flock -o /tmp/kayfabe-fastguest.lock`; one QEMU at a time; after any stop IOMMU group 11 must read `DMA-FQ`, `pgrep qemu-system-x86` empty, `rm -f /tmp/kf-stop-winprod`; keep `/var/lib` above 100 GiB (112 GiB after deleting three finished disposable overlays 1620/1630/1650 on 2026-10-13; never delete `baseline/`, `owner-state/` or `boundary-kayfabe-1380`).
No Vast instances were running (2026-10-13). Backups made 2026-10-13: `backup/stash-ci-repair-20261005`, `backup/host-kayfabe-int-master-20261008` (old, probably useless), `claude/host-tmp-scripts-20261013`. Other projects (nvkvm-pv, nvkvm-steamos, vast-windows) have dirty/unpushed work on this machine and the host; not touched.
