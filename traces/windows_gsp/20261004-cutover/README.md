# First actual Vast native cutover

**STATUS: RESEARCH, 2026-10-04 17:13 UTC. Disk writing began; native Windows boot/SSH is not yet established.**

Owned instance 54159260, RTX 3060, 150 GiB whole /dev/vda. The preparation harness cleanly shut down Windows, compacted and checked the image, compared it identical to staging and exited 0 at 17:01:28 UTC. Compact QCOW2 size 6,242,498,048 bytes; SHA256 `1fe10630f73d36f01387b51ed73db86833a13f7a4fd867e854d83c6c6580aac7`. Virtual size exactly matches the whole boot disk: 161,061,273,600 bytes.

The arm operation and target-side verification checked disk/root identities, staged kernel, every embedded runtime executable/library, manifest, GRUB entry and one-shot selection. The controller preserved the ready manifest and pinned Windows public host keys privately before reboot. Only text/JSON verification is retained here.

Root issued the guarded reboot at 17:07:02 UTC. Provider serial output confirms the RAM init booted, verified the target, copied the compact image to RAM, unmounted the source and began the irreversible write. The first flasher version has no intermediate write/readback progress. A final VERIFIED line and successful pinned Windows SSH are still required; these initial logs alone do not prove completed replacement.
