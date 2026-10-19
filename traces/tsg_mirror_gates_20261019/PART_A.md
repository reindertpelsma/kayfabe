# M1 Part A: Linux guest inventory and provisioning constraint

STATUS: PARTIAL, 2026-10-19. Labels: MEASURED / CODE READ / INFERRED.

MEASURED: inspected 62 Linux qcow2 candidates read-only, including the two bench base images and Linux descendants discovered under the designated data/image roots. All 62 contain gcc, libcuda and NVIDIA kernel modules. None contains the documented graphics sources/binaries, app-matrix bundle, LLM venv, glslangValidator, Xvfb or VirtualGL. Fifteen have vulkaninfo/vkcube, sixty have glxgears, and forty-six have Xorg. These are inventory counts, not successful executions. See `inventory-summary.json`; raw per-image inventory stays in the lane's host directory (`offline-inventory-v4.jsonl`). A separate check found the desktop seed's NVIDIA ICD under `/etc/vulkan/icd.d`, so the initial `/usr/share`-only ICD count must not be read as driver absence.

MEASURED: prepared an M1-owned qcow2 overlay plus a private copy of the benchmark guest SSH key. No existing disk was modified. The overlay has not booted and is NOT a provisioned gates image. Host main filesystem had 97 GiB free; dev root had 53 GiB free. No deletion of another lane's files is needed.

MEASURED: mirror binary MD5 `48ab1b70c2cf2c26d2c1287bf7311a9a`, base candidate-5 MD5 `c2ff1a02ed33a39e4b26d2dd002e62f6`; both match the supplied artifact records. Candidate-5 broker is present under the candidate-5 artifact's `cursor7` directory; the shorthand standalone cursor7 directory is absent. Default host static-video and app-bundle prerequisites are absent. This inventory does not establish that no alternative ffmpeg/toolkit installation exists anywhere on the host.

CODE READ: the provisioning chain is `provision_bench_tree.sh` Track B (stock QEMU/slirp, stock NVIDIA installer) -> `provision_guest_gfx.sh` -> `provision_guest_display.sh`. `scripts/apps/build_bundle.sh`, `setup_side.sh`, and `provision_guest_apps.sh` provide the app matrix. `provision_guest_llm.sh` provides the separate LLM model/runtime. The existing desktop seed avoids reinstalling the OS/driver but still needs the graphics and application provisioning.

CODE READ: documented gfx/app/display provisioners explicitly refuse a concurrent QEMU. Common instructions require the shared GPU lock for **anything that boots a VM**. Lane Part A requires provisioning before Part A is ready, while owner run 3009 retains that lock until Part B. MEASURED: run 3009 is still alive and its launcher reports READY. No provisioning VM was launched and no guard was bypassed.

## Stop/options

CODE READ: the instruction-conflict rule says STOP and write down options. The next provisioning boot cannot satisfy the specified ordering and shared-lock rule together. The exact proposed exception has been requested; no answer is assumed.

1. Recommended: authorize gracefully stopping run 3009 before provisioning, then run the documented stock-QEMU provisioning under the shared lock at the start of Part B, with all host writes relocated to M1's directory. Retain the 04:30 CEST deadline and release between stages.
2. Keep the ordering literally: stop M1 with all hardware gates UNRUN, provide this inventory and the prepared empty overlay, and arrange a pre-provisioned image before a later gate session.

INFERRED: attempting an unreviewed offline package-install/chroot flow, ignoring pgrep, or booting without the shared lock would evade the stated constraints; none is a substitute for resolving the ordering conflict.

MEASURED harness incident: bulk read-only mounting initially failed because the partition device node did not exist yet after NBD attachment (`mount` rc 32). Waiting for the specific udev partition event, with a bounded failure, completed all 62 inspections. No product timeout/poll constant changed. Every inspection unmounted its filesystem and disconnected its own NBD attachment; the device is now released.

CODE READ: existing fake-host unit tests include `sixty_four_subcontexts_are_accepted`, `the_sixty_fifth_subcontext_is_refused_by_name`, and `the_refused_sixty_fifth_leaves_the_map_at_sixty_four`. Targeted `tsgmirror` tests are being checked separately; even a pass does not establish hardware cap behavior or 8-channel guest execution.
