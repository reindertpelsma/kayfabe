# Proprietary test images

STATUS: LIVE, 2026-11-15 (policy). This page states how the maintainers handle the private Windows test images and what the repository does and does not contain. It is not legal advice and says nothing about the licence terms of any application.

## What a proprietary test image is

Kayfabe is tested with real applications, among them commercial design, video and 3D applications and benchmarks. A **proprietary test image** is a private Windows disk image that the maintainers build themselves to run those applications under kayfabe, offline, in the project's tests.

## What the repository contains and does not contain

- The repository contains **no** proprietary software, installers, licence keys, account data or disk images. It contains the scripts, the answer file, the verification code and the documents that describe how such an image is built and checked.
- A maintainer-built image is stored in a private object store. It is never published, never committed to git and never served publicly.
- A rented test box receives an image only through a time-limited, authenticated download.
- The maintainers' images are not shared with anyone. This includes contributors, reviewers and other people who run kayfabe's tests.

## Who can reproduce the proprietary-application tests

Anyone. You build your **own** image with the how-to [Recreate the proprietary Windows image](recreate-the-proprietary-image.md), using **your own** accounts and licences, and run the same offline tests against it. Contributors do the same. The how-to links only official download pages and never a protected installer.

Standalone use is supported: someone who builds an image themselves can run the same lane. That lane is not gated (owner ruling, 2026-11-15).

## How the images are used

- **Offline.** A run uses a user-mode network with `restrict=on`: the guest has no internet. Only the SSH forward from the host exists.
- **Deterministic clock.** The clock starts at the guest time recorded at the image's last clean shutdown plus a small gap derived from the image's seed, and never starts earlier than that. The reason is reproducibility: boot behaviour, timestamps in results and application and certificate timers are the same on every run. The install session that builds an image uses the real clock, because TLS to the vendors' sites needs correct dates.
- **Fixed hardware identity.** SMBIOS UUID and serials, MAC address, disk serial and GPU UUID of the virtual machine are derived from one seed per image, so that runs are repeatable.
- **AMD host.** The image uses an AMD CPU profile, so the host CPU vendor must match it. Intel hosts are not supported by that image. A second image, built on an Intel host, would be needed. On 2026-11-15 the maintainers measured an Intel host that presented the AMD vendor to the guest: it hung in Windows Boot Manager.

## Why the sign-in state is revoked at the vendor before the image is distributed

A rented test box is not trusted, and whoever controls it can read the image delivered to it. The procedure is: back up the disk and its state first; then, on a separate copy of the disk, sign out of every account, unlink the device, change the password and end all sessions at the vendor. Any sign-in state still stored in the backup is then revoked on the vendor's side and cannot be used to sign in anywhere. This is the procedure and the design intent. The tooling cannot see vendor-side state, so nothing in the repository verifies it. The how-to gives the order in *Why the sign-in state is revoked at the vendor before the image is distributed*. The reasons for the whole arrangement are isolation (the guest is hostile and rented hosts are untrusted), reproducibility (no network dependence and a deterministic start), and a private store so that nothing proprietary is redistributed.

## How the maintainers' images reach a test box

The policy is a time-limited, authenticated download to the box, from the private store. The repository has a mechanism of that kind for other private test data: a Worker route that checks the box's run token and answers with a signed URL that lives at most 20 minutes (`docs/DOWNLOADS.md`, *SPECviewperf private copy*). CODE READ at revision `84b541cae` (2026-11-15): the list of objects that route will serve holds no proprietary image, and no script in the repository fetches one. So the delivery of an image to a box is the maintainers' policy and is not shown by code in this repository.

## What the open lanes use

The open lanes need none of this. A Windows 11 Enterprise LTSC evaluation guest with internet access runs only open or freely downloadable software, plus SPECviewperf, SPECworkstation and the Phoronix Test Suite as separately obtained benchmark packages. Proprietary applications run only in the offline images described above. This is the owner's scoping ruling of 2026-11-15 (`docs/OWNER_RULINGS.md`); that ruling records that whether today's code already meets every part of it has not been audited.

## What is never published

Never in the repository, in a release, on the site or in a public cache: a proprietary image or any part of it, an installer, a licence key, an account name or password, the contents of an image's state file (its TPM state, SSH key pair and account password), and the names and locations of the maintainers' private objects.

Verified on: policy text from the owner's decisions of 2026-11-15; code statements read at `84b541cae` (`ci/vast-broker/src/benchdata.js`, `scripts/bench/windows/propimage.py`, `scripts/bench/windows/propimage_check.py`). Not verified end to end: no one has followed the how-to from scratch, and the delivery of an image to a box is not exercised by any code in the repository. <!-- src: docs/howto/recreate-the-proprietary-image.md; docs/design/V3_WINDOWS_PROPRIETARY_IMAGE.md; docs/OWNER_RULINGS.md section 1 of the 2026-11-15 release-gate rulings -->
