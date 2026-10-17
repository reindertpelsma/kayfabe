# AD102 Linux590 CUDA/graphics/Cinnamon broker PASS after explicit modesetting profile

**STATUS: LIVE, 2026-10-16 23:40UTC. Completed, all GPU VMs stopped.**

Own56302930, actualAD102/PCI2684, host595.91.07, source9d6f210ba1dc6ab0edbc49ff265101091e8b08f1,
privateQEMU SHA cc2a86a6dec3ec6bed91d9cd6e1963a1deadf2d75a768e9e3dada91559e34668.
Actualguest590.48.01 module/nvidia-smi/libcuda.so.590.48.01, kernel6.8.0-142-generic.
Native595CUDA4 and nativeNVIDIAgraphics completed before guestapps. These are fixed-host
595/guest590 compatibility results, not a native590-host test. Raw59030/30 at180s is
separately committed4ff55de0d in ../linux590_regression/.

MEASURED initial headlessprofile: CUDA4/4 PASS and all10Vulkan/EGL/GLX hashes match
nativeNVIDIA. GFXS2–5 PASS; S1FAIL because nvidia_drm already loaded with modeset=N.
modprobe modeset=1 cannot change an already loaded parameter. No graphics-content
failure or newhostXid occurred; ownedQEMU stoppedexit0 at23:26:36. Preserve that
strict all5 failure as a configuration negative, not a fullsuitepass.

Correction: interactive desktopprep sets nvidia-drm.modeset=1/fbdev=1 in GRUB.
First preparedprofile check used an unprivileged directsysfsread and failedPermissionDenied
before anyapp ran; that harnessattempt is retained separately. The corrected readonly
check uses sudo -n (same as the existing gfxsuite). Reuse alreadyprepared desktopimage,
noGPUcode/binary/profilechange between those twoprepared attempts. Falsifier declared
before correction: actualY stillfailsreadiness, mismatchedcontent or blankbroker.

MEASURED final QEMU146107: actualKF3environment onlyKF3_REV=9d6f210b (binaryselector),
no otherKF3flag, source/privateSHA/argv asserted. LoadedmodesetY and GRUBarguments1
verified; actual590module/libcuda/kernel printed. CUDAcup2/cup3/cup8/cup8bench PASS4/4;
NVIDIAhardwaregraphicsS1–5 PASS and all10hashes equal the earliernative595reference.
Cinnamon1/Xorg1/NVIDIAOpenGL, actualX11SHMwindow1920x1080/11708colors,
pixelSHA5d758ac02ee42d8102c8d943317f244db1c8107c0a26904a1e9decdabdb397bf.
Guest fliptimeouts0/Xid0; hostXid10→10 exactsnapshotsunchanged.162/162USERappchannelbirths,
5CUDAthreadpostures,0heartbeatwarnings in thisappVM. Successfulbrokerexit0 and owned
QEMUcleanup at23:36:40; noSIGKILL. No software-renderer substitution.

Material limits retained:18host displaySTALL messages and10guest nvAssert lines.
This is functionalpresentation and contentparity, not latency/warningclean acceptance.
H-L3 functionalsuite notfalsified after correctprofile. AD102 Linux580/590/595 now
have raw/CUDA/graphics/broker passes, with exactrevisions/evidence separately recorded;
Windows581Code43 and additionaldies/builds keep fullfamilyacceptance incomplete.

Only screenedtext exported; rawimages/executables/frames never return. Logs>1MiB
are excerpts with rawsize/SHAmanifest. Identifiers/CRLF/trailingwhitespace normalized.
Allrelevanttext must be pushed before the planned sameownedrental nativeWindows581
reimage; remainingrawlogs/images will be erased by that conversion. Originaltimer
00:46UTC/hard00:51UTC2026-10-17 unchanged; no leaseextension or secondactiverental.
