# Report a problem

A report that can be acted on says what you ran, on what, and what the log said. This page lists what to attach and what to leave out. First read [Troubleshooting](troubleshooting.md): many messages say exactly what to change.

**A suspected security problem is not an ordinary report.** Do not open a public issue; use the private channel in [Security model](security.md).

Otherwise open an issue at <https://github.com/reindertpelsma/kayfabe/issues>.

## What to attach

1. **What happened, and what you expected.** The guest (Linux or Windows, which version), the program you ran, and the first thing that went wrong.
2. **Does it work without kayfabe?** If the same program passes on the host with no VM, say so: then the defect is kayfabe's. If you have a VFIO setup, say whether the guest works there.
3. **The QEMU command line.** Everything after the binary's name: from your launch script, or `ps -o args= -p <qemu pid>`. Include the `-device kf3-gpu,...` argument and the memory arguments.
4. **The kayfabe revision.** The line `KF3_BUILT <path> rev=<revision>` that the build printed, or the git revision of your checkout, and QEMU's version (`qemu-system-x86_64 --version`).
5. **The host driver.** `cat /proc/driver/nvidia/version`, the GPU model (`nvidia-smi -L`) and the kernel (`uname -r`).
6. **The guest driver.** Linux: `nvidia-smi` and `modinfo nvidia | grep -E '^(version|license)'`. Windows: the driver version from `nvidia-smi` or the Device Manager, and whether the display adapter shows an error code.
7. **Kayfabe's log.** QEMU's standard error, kept in a file: the lines that start with `kf3:`, `kf-host:` and `kf-rm:`, and anything around them. Attach the whole file if it is not huge; if it is, attach the first 200 lines and the last 200 and say so.
8. **The host's `Xid` lines.** `dmesg | grep -i xid` (or `journalctl -k | grep -i xid`) on the host, covering the time of the problem. A GPU fault does not show up in the guest.
9. **The guest's own log.** Linux: `dmesg` in the guest. Windows: the System log entries from `nvlddmkm` and `Display` (events 153 and 4101) around the time.
10. **The `nvidia-smi` output** from the host and from the guest, as text.

If a screen is involved, a screenshot helps. Say whether you used VNC or the display broker, and the broker's log.

## What to leave out

- Licence keys, product keys, passwords, ssh keys, access tokens and the TPM state of a Windows guest.
- The GPU's UUID and serial number: `nvidia-smi -L` and `nvidia-smi -q` print them. Replace the digits with `x` before you post the output. The model name, driver version and PCI address are fine.
- Host names, user names, IP and MAC addresses, and paths that name you or your organisation.
- Vendor installers, disk images and anything you may not redistribute. Name the program and its version instead.
- Private data in a guest's files or screenshots.

Read an attachment through once before you post it: logs often contain the command line of the program you were running.

## Make it small if you can

If you can, reproduce the problem with the smallest program, the shortest run and the fewest devices. Say how long it takes to appear, and whether it appears every time. A problem that needs a commercial application is still worth reporting; say which application and version.
