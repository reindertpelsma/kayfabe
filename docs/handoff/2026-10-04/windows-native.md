# Native Windows installer and GSP recorder

**STATUS: RESEARCH, 2026-10-04. Work in progress; no native Windows GPU success or
Windows GSP query capture yet.** The owner requested this lane after the candidate
2 handoff. It does not change the product merge requirements or claim a Windows
guest works through Kayfabe.

## Durable source

- Public standalone installer: <https://github.com/reindertpelsma/vast-windows>,
  `main`, local `/data/vast-windows`. It includes preflight, unattended nested
  Windows preparation, cold-boot/network/SSH checks, the same-disk RAM flasher,
  native NVIDIA/CUDA startup automation and an optional browser desktop helper.
- Recorder: this repository's `codex/windows-native-trace-2026-10-04` branch,
  `tools/windows-gsp-trace/`, local `/data/kayfabe-windows-native-20261004`.
  Source commits `4f93e0c9` and `e9612f4c` are pushed. The unsigned driver and
  collector were built locally from pinned official SDK/WDK inputs and copied
  outward to the rental; no rental executables were copied back.
- Linux captured-payload replay and bounded queue/parser tests passed. Passive
  sampling can miss startup traffic; a valid request/reply pair is required
  before treating query fields as evidence. Nothing supplies per-die constants
  to the product.

## Active rental and current stage

Owned instance **54159260**, local SSH alias **`vw-native`**, RTX 3060, about
98 GiB RAM and a 150 GiB whole boot disk. This session owns cleanup. Existing
instance **54049598** belongs to another ongoing lane; do not overwrite or
destroy it. Three failed/pending rentals from this lane were already destroyed
and their absence verified: 54157671, 54158787, 54160009.

The current Linux disk has **not** been armed or overwritten. Installation is
running under nested QEMU with its verified media cached. Inspect:

```sh
ssh vw-native 'tail -30 /root/vw-prepare.log'
ssh vw-native 'ls -lh /var/lib/vast-windows/staging.qcow2'
```

The first attempt exposed a WinPE device-enumeration issue: the unattended seed
on the second IDE channel was invisible. The public installer now attaches it as
USB optical media. The second attempt started at 14:43:41 UTC and visibly reached
Windows installation. Its generated image was over 6 GiB by 14:55 UTC; this is
progress evidence, not installation success.

Current preparation uses `--login-test-hold 900`: after two cold boots it writes
`/var/lib/vast-windows/login-test.json`, then waits for `login-test-complete`.
The controller must pin those public Windows host keys and perform an actual
authorized-key SSH login through the outer Linux SSH connection. No private
client key goes to the rental. The automatic checks honestly leave
`actual_authorized_key_login=false`; controller evidence is recorded separately.

Two coordinated workers are preparing to stage the native GPU task with its
research `-Defer` switch and the recorder into the existing Windows image via
QGA. The recorder needs Windows test signing enabled before the next boot. The
generic public installer keeps ordinary NVIDIA GSP policy and automatic native
GPU installation by default; research deferral and forcing GSP are explicit
options. Consult the latest worker/session state before repeating any staging.

## Next checks

1. Complete Windows setup, inspect firstboot and cold-boot evidence, stage the
   deferred GPU setup and recorder, and verify a real controller SSH login.
2. Release the final hold, require clean shutdown and exact-size/hash sealing,
   preserve the public host keys and useful text evidence off the rental.
3. Arm only this owned instance, then perform the Linux-to-RAM and RAM-to-native
   Windows reboots. The flasher was rehearsed under KVM/UEFI, including full
   readback and a 512-byte oversized-image prewrite refusal; actual rental GRUB
   and firmware boot remain to be exercised.
4. Reconnect to Windows as `vast`, verify the recorder loads and its API tests
   pass, then resume NVIDIA/CUDA setup with the recorder active. Validate a real
   CUDA kernel result and decode captured query traffic. Absence of a sample is
   not evidence that Windows omitted the query.
5. Save useful text evidence to GitHub after each run. Keep the rental only while
   actively used; destroy by its owned ID or explicitly hand off ownership.

The public native package profile currently targets supported Turing-and-newer
GeForce devices with driver 580.88 and CUDA 13.0.0. It does not yet cover all
datacenter GPUs. Broader die/driver validation and any derived Kayfabe fix remain
separate work after the capture mechanism is demonstrated.
