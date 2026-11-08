STATUS: RESEARCH, 2026-11-08. Bounded CPU/filesystem observations only; no bootstrap, app image, mount, chroot, VM, GPU, upload or deployment.

# LB1 chroot alternative evaluation

**CODE READ:** the [evaluation](../../docs/design/V3_APPS_IMAGE_CHROOT_EVALUATION.md) answers the owner's follow-up about independent root builds without KVM, offline-init compatibility, privilege needs, disk/time planning and level-1/level-2 reproducibility. It preserves the existing QEMU/cloud-init and bash paths. Repository inspection and recipe accounting used `1ef007e4fd6197547091e7d008def1a0d10b1860`; this revision already contains the original LB1 implementation.

## Question and falsifier before the probe

**RESEARCH:** would fixed filesystem UUID, directory hash seed and epoch suffice without full metadata normalization? The falsifier was an otherwise equal content tree producing different bytes after changing an older input timestamp. A narrower hypothesis was that fully normalized tiny trees could produce equal bytes on this one host; any differing normalized output would falsify it. Neither question establishes complete OS/app reproducibility across hosts.

**MEASURED, 2026-11-08 ([probe.json](probe.json)):** the [exact runner](probe.py) executed with uid/gid 65534 and zero effective capabilities using e2fsprogs 1.47.2. It creates one disposable 32 MiB regular-file image at a time, checks it read-only, records its hash and deletes it. No installed package, namespace, mount, VM or host configuration was changed. Filesystem tools and their local executable hashes, formatter configuration, epoch, synthetic IDs, inode diagnostics and timings are recorded. The runner SHA in the JSON matches the committed runner.

| Variant | Observed SHA relationship to normalized A |
|---|---|
| Normalized A | Reference |
| Normalized B, reverse source creation order | Equal |
| Normalized A repeated | Equal |
| One file with older mtime, unchanged content | Different |
| No fixed directory hash seed | Different |
| No fixed epoch | Different |
| No fixed filesystem UUID | Different |

**MEASURED, 2026-11-08 ([probe.json](probe.json)):** all three normalized images have SHA256 `875fe5da3d1f2ec4d8aed88279c0d3218cf77406a80bb9c68d2a057283981c29`. The seven content inventories agree; the four negative controls have different filesystem hashes. All seven `e2fsck -f -n` invocations return zero and preserve the hashes. Every image is 33,554,432 logical bytes and allocates 4,603,904 bytes on this host. Total recorded probe time is 10.294097 seconds, including formatting, hashing and read-only checks; it is not an apps build-time estimate.

**INFERRED from this narrow observation:** byte equality is achievable for this normalized fixture/tool combination. Fixed identifiers and epoch alone do not force older metadata to the same value. The result says nothing about different machines, formatter/library versions, root-owned package trees, xattrs/capabilities, kernel/initramfs generation or real compilation outputs. Fixture files are uid 65534, not a completed bootable Ubuntu filesystem.

**CODE READ:** the upstream source path that clamps newer times but preserves older ones is [e2fsprogs 1.47.2 create_inode.c](https://github.com/tytso/e2fsprogs/blob/v1.47.2/misc/create_inode.c). More primary references and all proposed qualification falsifiers are in the design evaluation.

## Repeating only the bounded probe

**CODE READ:** use a writable scratch directory and output path; Python 3.11+ and already-installed `mke2fs`, `e2fsck`, `debugfs` are required. No sudo, loop setup, mount or QEMU is needed. Individual argv invocation:

```text
python3 traces/apps_image_chroot_eval_20261108/probe.py --scratch /path/to/owned/scratch --out /path/to/owned/probe.json
```

**DESIGN:** examine the resulting observations, including any differences; the runner records outcomes rather than turning an unexpected difference into an unsupported production pass. Do not extrapolate its tiny-image timings to complete builds. Peak logical image capacity is 32 MiB, plus small fixture files and metadata; image files are deleted after each variant.

## Accounting, validation and limits

**CODE READ:** [recipe_audit.json](recipe_audit.json) records sums and the source recipe digest: 2.72 GiB direct open payloads excluding the cloud base, 1.85 GiB current transient compiler inputs and 4.79 GiB full separate official inputs, before apt downloads and unpacking. The disk/time worksheet in the design is explicitly INFERRED. Historical R5 timings are cited there as historical observations only.

**CODE READ:** this follow-up adds research documentation and this standalone observation runner, not a production backend, verification rule or release gate. Existing image tests and 61 two-killer mutants are unchanged; their earlier evidence remains [the original LB1 record](../apps_image_recipe_20261107/README.md). No new production mutants apply to this evaluation. Complete bootstrap, cross-host inventories/bytes, boot compatibility, resource peaks and GPU qualification remain coordinator work. Fresh documentation/claim/private-reference/UUID/workflow gate exits are recorded in `gates.json` after validation.

**MEASURED, 2026-11-08 ([gate exits](gates.json), [validation](validation.json)):** all six local hygiene commands return zero. Documentation freshness covers 213 documents with zero new findings; claim attribution has zero new sites; release claims have 133 rows, zero errors and three existing warnings; private references are clean; GPU UUIDs have zero new/stale findings across 179 baselined files; workflow lint is clean. No baseline was edited. Black accepts the observation runner, its recorded SHA matches, and the staged diff passes whitespace and private-key/GitHub/AWS/Slack/bearer-literal pattern checks. These pattern checks are not an exhaustive secret audit.
