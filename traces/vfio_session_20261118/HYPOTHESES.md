# Hypotheses stated BEFORE each row (2026-11-18). Rule: kf3 FAIL + VFIO PASS = kayfabe defect; both FAIL = app/harness; both PASS = flaky elsewhere (Vast die / nested KVM).
## Row 1: Resolve (image A, row `resolve`, 4070). Seen on 2 kf3 RTX 4090 Vast boxes: Resolve.exe crash in nvoglv64.dll about 3-4 min after project load (3 of 3).
H1 (kayfabe defect): the crash is caused by something kf3 does or lacks -> VFIO on the 4070 keeps Resolve alive well past 6 min after project load (soak 600 s), no Application Error 1000 (nvoglv64.dll).
Falsifier: Resolve.exe crashes in nvoglv64.dll on VFIO in >= 1 of 3 attempts -> app/driver, not kayfabe.
Prior MEASURED on this pc (rev 6bfa2dbc): VFIO 2 of 2 alive 935 s, kf3 1 of 1 alive 1118 s -> both-pass expected (Vast-only). Caveat: the row's UI driver fails at the Deliver Format step on both platforms (harness), so only the load/idle window is a control.
## Row 2: Wukong benchmark (image A, 4070). kf3 36 FPS vs VFIO 93 FPS (earlier); kf3 LiveKernelEvent 141/117 / nvlddmkm 153 TDR.
H1: TDR and low FPS are kf3-caused -> VFIO 3 of 3 results, 0 TDR (no LKE 141/117, no nvlddmkm 153), FPS near 93; kf3 reproduces slower and/or TDR.
Falsifier: VFIO shows a TDR/LKE event in >= 1 of 3 attempts -> app/driver, not kayfabe.

## Finding 2026-11-18 04:05Z (after row 1 VFIO): late udev unload of nvidia-uvm after a clean restore (MEASURED + INFERRED kept apart)
MEASURED (ev/s1_after_restore_uvm_hang.txt, ev/s1k_cuda_ready.txt, PC_ACTIVE.log 04:05:03Z): session s1v restored cleanly at 04:02:16Z (every rmmod/modprobe rc=0, nvidia-smi OK, gdm active, audio function back on snd_hda_intel).
About 1 to 3 minutes later `/sys/module/nvidia_uvm/initstate` = going, refcnt -1, and a `/sbin/modprobe -r nvidia-uvm` child of a udev-worker sat in D state at
`rcu_barrier <- kmem_cache_destroy <- _uvm_tools_destroy_cache_all <- uvm_tools_exit <- uvm_exit <- delete_module`, new instances every few seconds; a kernel worker was in wait_rcu_gp for >2 min.
A kf3 session started in the same minute got `cuInit 999` (ev/s1k_cuda_ready.txt, 04:03:16Z) and was killed by us in its settle wait (no boot).
The rule that runs the unload (/usr/lib/udev/rules.d/71-nvidia.rules, identical copy in /lib/udev/rules.d):
  line 18: ACTION=="add",    DEVPATH=="/bus/pci/drivers/nvidia", RUN+="/sbin/modprobe nvidia-uvm"
  line 19: ACTION=="remove", DEVPATH=="/bus/pci/drivers/nvidia", RUN+="/sbin/modprobe -r nvidia-uvm"
INFERRED (not shown): the remove event of the driver (our rmmod nvidia at bind, or the restore's reload) is processed late by udev and its `modprobe -r nvidia-uvm` runs against the freshly reloaded, in-use uvm: the unload races the reload and blocks in rcu_barrier. This would also explain the 2026-11-17 wedges (modprobe -r nvidia-uvm in D) and the earlier "cuInit 999 right after a restore" (three kf3 sessions).
Falsifier: after the fix (udevadm settle + wait for no nvidia modprobe/rmmod running and uvm absent-stable 60 s or live with refcnt >= 0 stable 60 s) a restore still leaves uvm going > 120 s.
ACTION: SysRq ladder (s, u, b) executed 04:05Z with coordinator approval; host config unchanged (no udev drop-in).
