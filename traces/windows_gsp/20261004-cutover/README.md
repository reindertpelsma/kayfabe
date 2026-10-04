# First actual Vast native cutover

**STATUS: RESEARCH, 2026-10-04 17:40 UTC. Full readback, native Windows boot, pinned SSH and recorder API tests passed. NVIDIA/CUDA and GSP capture remain separate checks.**

Owned instance 54159260, RTX 3060, 150 GiB whole /dev/vda. The preparation harness cleanly shut down Windows, compacted and checked the image, compared it identical to staging and exited 0 at 17:01:28 UTC. Compact QCOW2 size 6,242,498,048 bytes; SHA256 `1fe10630f73d36f01387b51ed73db86833a13f7a4fd867e854d83c6c6580aac7`. Virtual size exactly matches the whole boot disk: 161,061,273,600 bytes.

The arm operation and target-side verification checked disk/root identities, staged kernel, every embedded runtime executable/library, manifest, GRUB entry and one-shot selection. The controller preserved the ready manifest and pinned Windows public host keys privately before reboot. Only text/JSON verification is retained here.

Root issued the guarded reboot at 17:07:02 UTC. Provider serial output confirms the RAM init booted, verified the target, copied the compact image to RAM, unmounted the source and wrote the disk. The first flasher version has no intermediate write/readback progress. The final serial output reports `Images are identical` and `VERIFIED`, followed by reboot at kernel uptime 654.823 seconds. Firmware then started the generic loader on the same disk.

Windows reported boot time 17:18:17.500 UTC. Public-key SSH succeeded at 17:18:41.349 UTC with the Windows host key pinned before cutover. Windows 11 Enterprise LTSC Evaluation build 26100 sees the RTX3060 PCI functions. The NVIDIA/CUDA task was still deliberately deferred at this observation; the pre-install Basic Display Adapter problem43 does not establish NVIDIA driver failure. The existing MSVC observer then passed its Windows API tests with zero failures on the native OS.

`provider-flasher-current.log` is the earlier partial observation. The sanitized final serial log is `provider-flasher-complete.log`, successful SSH/OS/device observations are in `native-windows.json`, and `native-recorder-smoke.stdout` records the native API/process smoke. The public standalone installer's [complete evidence record](https://github.com/reindertpelsma/vast-windows/tree/main/docs/evidence/native-cutover-20261004) describes the one validated rental profile and source provenance. No GSP query pair or CUDA workload is established by this record.
