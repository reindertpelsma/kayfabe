STATUS: LIVE, 2026-10-23 — native baseline complete on own lease 57310491.

2026-10-23 MEASURED GB202 / RTX5090 with open host595.91.07 and frozen product6e3fd3fd6: hostpreflight SOUND, gates9/9+10PASS, raw30/30, primary CUDA4/4, GR/CE15/15 using corrected fixture618f04318. Receipts are under lease_metadata/. QEMU and workload source hashes are preserved; the rebuilt fastguest595 was not run, so Linux guest validation is UNRUN.

2026-10-23 MEASURED exactly five intentional Xid31 controls with source/PID/channel/VA/type matching and positive recovery: defer PID92537, ENGINE CE0 / HUBCLIENT_CE2 or CE3, PTE virtual reads at 0x60/62/64_00010000 on channels2/3; missing-page PID92594, ENGINE GR_HOST0 / HUBCLIENT_ESC0, PDE virtual write at 0xa_11000000 on channel3. The actual source FAULT_MARK is victim_release, not sem-release or probe-launch-dma; bystander before and after are Landed. No native32 or unrelated Xid is admitted; full strings remain ordered in native_receipt.json.

2026-10-23 MEASURED post-read checkpoint passed and Windows standard-VGA LTSC installation began03:06:13UTC, PID98482. The tuple-vs-JSON correction bd3d54172 preserves exact ordered equality. Secondary guard is ready and the pidfd disposable-child smoke passed. Windows580.88 bind/ON/Code0/43/TDR/hold/B1 remain UNRUN at this native checkpoint.
