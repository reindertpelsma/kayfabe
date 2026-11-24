# traces/vkcts_host_20261117 — native build and host runs of the open Vulkan CTS row

STATUS: LIVE, 2026-11-17 (evidence, text only). Run: one Vast CUDA container (not the KVM image, no kayfabe, no kf3 in it: the native baseline), GeForce RTX 3060 (GA106, 12 GB),
driver 580.159.03, image `nvidia/cuda:12.6.3-devel-ubuntu22.04`, label `kf-openlin-vkcts2-171803`, rented 2026-11-17 18:03Z and destroyed 18:25Z (0.04 USD; the first box of the
session, `kf-openlin-vkcts-171400`, was destroyed by the cost guard before its build result was read and left no evidence, 0.36 USD). Tool revision: branch
`claude/open-vulkan-cts-20261117` (`scripts/apps/vkcts/`, unchanged between the runs below), CTS pinned at `0d18b347943e5e843529a05c95220e27fe30c538` (`vulkan-cts-1.4.7.1`).

| file | what |
|---|---|
| `box_facts.txt` | GPU, CPU quota, toolchain, the apt packages installed before the build |
| `build_manifest.json` | what `vkcts.py build` recorded: pins, cmake arguments, jobs, `fetch_s` 237, `build_s` 341, binary 77,488,128 bytes, compiler |
| `host_run1_results.tsv` | the per-case results block of the first host run (291 cases: 189 pass, 102 not supported, 0 fail) |
| `host_run1_summary.txt`, `host_run2_summary.txt` | summary, device, run time and GPU-use verdict lines of two host runs |
| `diff_run1_vs_run2.txt` | `vkcts.py diff --host run1 --guest run2`: 0 candidates, equal digests `70d5ed8f0ca07431` (the host-versus-host control) |
| `row_via_run_apps.txt` | the matrix row as `scripts/apps/run_apps.sh host vkcts` reports it (`verdict=PASS`, same digest) |

Commands (on the box, root, untrusted, no credentials, the repository cloned from GitHub): `apt-get install cmake ninja-build libvulkan1 vulkan-tools libxext6 libx11-6 libegl1`,
`python3 -I scripts/apps/vkcts/vkcts.py build --root /root/vkcts`, `python3 -I scripts/apps/vkcts/vkcts.py run --side host --artifact /root/vkcts/current --out /root/run1`.
The first build attempt stopped at the CTS's own video fetch script because it was run isolated (`-I`); fixed in the tool and not part of this evidence. No UUID, address or
account detail is in these files. The guest side of the row has not run; see `docs/design/V3_VULKAN_CTS_ROW.md` section 10.
