# Resolved USERD correction: exact Linux hardware regression

STATUS: ANSWERED, 2026-10-12. Normal Linux regression, not forced-relay validation.

Full source6ff5aaf293e9fc04ac8780983f26dc04d0f3b735 on trusted172.22.1.20,
RTX4070/host595.91.07. Gates9/9+gate10PASS, USERbirth12/12/refused0; exact rebuilt
QEMU/client/fresh fastguest. Fast30/30, crashes0, NOTRUN0, exit0. Start18:02:40UTC,
fast exit18:07:13UTC. Five Xid31 correspond to intended negative virtual accesses;
noXid32. Binary SHA and complete per-arm scoreboard retained in result/.

Fresh disposable Linux desktop child: Cinnamon/Xorg, NVIDIA580.159.04 RTX4070,
gears59–60FPS. PASS_WITH_WARNINGS due startup assertions and flip timeout, no
hostXid. Not a render-parity claim. Stopped18:08:04UTC, noQEMU/DMA-FQ. See desktop/.

Own run1550 disposable disk removed only after every readable qcow2 header was
checked for dependents; personal1380/baseline/resume images protected. Cleanup
report retained. Source correction1919GPU-freePASS/source-reviewPASS; subsequent
Windows1560 is a separate diagnostic. Source-zero migration still unverified.
