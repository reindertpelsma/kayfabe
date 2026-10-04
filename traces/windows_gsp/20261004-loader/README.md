# Windows recorder kernel/API validation

**STATUS: RESEARCH, 2026-10-04. Kernel/API tests passed; no GPU was assigned to this nested Windows VM and no Windows GSP traffic was captured.**

Owned Vast instance 54159260, Windows 11 Enterprise LTSC 2024 build 26100, nested KVM, 8 GiB RAM. Test signing was active; Secure Boot and VBS/HVCI were inactive.

The combined LLD diagnostic image (`9191f961588bc0b17c70dc3cb8cce03ab75dba5fcc796fda8c4748c9287dc021`, unsigned SHA256) loaded, reached DriverEntry stage 5, passed the Windows API suite with zero failures, and unloaded successfully. The loader probe matrix is in `tools/windows-gsp-trace/LOAD_TESTS.md`.

The independent MSVC normal observer from source `351d5b7f425c7515404f058b0fa1f33602905e17` was then staged from trusted GitHub CI artifacts. Its exact compiler/kit/artifact provenance is in `msvc-build-info.json`. Installation, local test signing and verification, SCM load, API suite, service restart/status and service stop passed. The API suite checks statistics/worker progress, invalid IOCTL and lengths, exclusive/child-path handling, restricted-token denial, and worker stop. It stops the worker; restart before capture.

Text output was copied back through pinned public-key SSH. No target executable or crash dump was copied back. A final Windows readiness check passed all 25 provisioning/network/SSH/disk/boot checks after the recorder tests. The native GPU installer remained deferred, with reboots held. These results do not prove queue discovery on Windows or a usable query request/reply pair.
