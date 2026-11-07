# Windows image capture skeleton (S1), 2026-11-07: evidence index

STATUS: LIVE, 2026-11-07. Text evidence of one run on Vast instance 59574978 (destroyed) and the dev box. The analysis, with MEASURED and INFERRED kept apart, is
`docs/design/V3_WINDOWS_IMAGE_CAPTURE.md`. No image, no screenshot, no key and no presigned URL is in this directory; the box address is redacted to `<box-ip>:<port>`.

| file | what it is |
|---|---|
| `PREREG.md`, `RECOVERY_PLAN.md` | written before the first measurement and before the first reboot into the UKI |
| `facts_before_swap.txt` | `esp_swap.ps1 -Mode Facts` on the instance (Secure Boot, firmware entries, ESP, disks, NIC, TPM, BitLocker, edition, drivers; language files of the ESP listing trimmed) |
| `swap_out.txt`, `restore_out.txt` | the swap (`ESP_SWAP_DONE`) and the restore (`ESP_RESTORE_OK`) |
| `uki_boot_serial.txt` | the Vast serial console of the UKI boot (`KFUKI` lines, host keys, `READY`) |
| `uki_build.log`, `selftest.log` | the final builder's last lines; `uki_selftest.sh` run (`UKI_SELFTEST_PASS`) |
| `cap2.run.log`, `dd_run_summary.txt`, `density_samples.txt` | the parallel `dd-qcow2` run stopped at 36 chunks, and compressed sizes of sample 1 GiB chunks |
| `cap3.run.log`, `capture3.log`, `ntfsclone_save_tail.txt`, `partition_table.txt` | the `ntfsclone-qcow2` capture (stage times, stream sha256 on both ends) |
| `sparsify.log` | in-place freeing of all-zero clusters |
| `boot1.run.log`, `boot2.run.log`, `qemu_cmdline.txt`, `guest_facts_qemu.txt`, `guest_facts2.txt` | the two boot tests of the copy in the CI shape and what the guest reported |
