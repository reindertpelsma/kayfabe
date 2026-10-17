# Windows581.51 public ABI and package prerequisites

**STATUS: LIVE, 2026-10-16. MEASURED CPU/package acceptance only; hardware UNRUN.**

The exact published pair is Windows581.51 and Linux580.94.02, Vulkan beta
release2025-10-03. NVIDIA's public580.94.02 nvBldVer.h names Windows branch
VK580_65-5 and the same changelist36629274. The compiled kf-abi probe accepts
linux_twin("581.51","VK580_65-5",0) and versions::table_for("580.94.02").
This proves the identity/table lookup, not a complete GSP exchange or guest pass.

Primary sources:

- https://developer.nvidia.com/vulkan-driver
- https://raw.githubusercontent.com/NVIDIA/open-gpu-kernel-modules/580.94.02/src/common/inc/nvBldVer.h
- https://developer.nvidia.com/downloads/vulkan-beta-58151-windows

The full public Windows package is910810544bytes, SHA256
cb04aa1e2fbb977ee3911ab2cf51a42351f8126ba991bc9308ce6bf0ba96e83a.
Its archive passed a full CRC test. Eight extracted INFs mention AD102;
nv_dispui.inf provides generic PCI10de:2684 support and actual DriverVer
32.0.15.8151. The package was copied outward to the rental but never installed.
No executable or raw trace was returned. Canonical source URL is preserved;
ephemeral public-CDN query tokens are omitted.

Next run: separate child overlay of the stopped580 baseline, verify vendor
signatures and actual8151binding, then cold boot with ABI580.94.02 and the same
19production flags. Require the full Windows suite and >=420second zero-TDR
hold before calling this a second passing Windows build. The AD102 app run
finished near the lease cap, so no partial driver install was started.
Windows610.88 remains exact-identity unsupported as documented separately;
581.51 does not change that conclusion or satisfy the requested610.88 cell.
