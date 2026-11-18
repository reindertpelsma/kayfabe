# Pre-registration: Windows image capture skeleton (S1), 2026-11-07

STATUS: LIVE, 2026-11-07. Written before the first measurement on the rented instance (instance 59574978 had been rented, its Windows
install was still running, no command had been run on it). Each hypothesis states the result that would prove it wrong. Results are
recorded in `README.md` of this directory and in `docs/design/V3_WINDOWS_IMAGE_CAPTURE.md`, with MEASURED and INFERRED kept apart.

| id | hypothesis (INFERRED from the windows-on-vast README and the specnative lane's boot logs) | falsifier |
|---|---|---|
| H1 | The Vast KVM firmware boots Windows with Secure Boot OFF, so an unsigned UKI can start | `Confirm-SecureBootUEFI` prints `True` |
| H2 | The Windows ESP (the log shows 0x32000 sectors, 100 MiB) has room for two copies of a 26 MB UKI next to the backups | ESP free space below about 52 MB |
| H3 | After the swap and a reboot, the UKI answers ssh on the SAME public ip and port within 5 min | no ssh in 5 min (the UKI's own recovery timer then restores Windows at 15 min) |
| H4 | `dd | zstd` of the raw disk, pulled by the dev box over that ssh, runs at 40 MB/s or more of raw disk | below 40 MB/s |
| H5 | After `esp_restore.sh` and a reboot, Windows boots and answers ssh on the same ip and port within 10 min | not within 10 min |
| H6 | The captured disk, converted to qcow2, boots in the CI QEMU shape (q35, OVMF, swtpm, virtio-blk, virtio-net) to a state where ssh answers, with no preparation | INACCESSIBLE_BOOT_DEVICE, a recovery screen, or no ssh in 15 min |
| H7 | A raw copy compresses to less than a third of the disk size, because the free space of a fresh install is mostly zeros | zst/raw ratio above 0.33 |
