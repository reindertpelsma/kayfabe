# Integration revision apps/ladder run on a vast box, 2026-10-10

STATUS: RESEARCH, 2026-10-10. Partial run, stopped by the owner deadline (box destroyed 03:3x CEST).

Box: vast instance 55112711 (own id), RTX 3060 (GA106), 3 vCPU, 24 GB RAM, nested KVM, host driver 580.159.04 installed by provision_host_driver.sh.
Provision log: prov.log (clone of branch integration/windows-20261010 = a6587d6a at clone time; the requested revision 6fafcc6e is two commits older).

Measured:
- a6587d6a (head of integration/windows-20261010, includes the 2e10a0c7 carve-out fix): v3_gates 9/9 (gates.log), fast suite 30/30 (apps_a6587d6a_suite.out), CUDA ladder guest cup2/cup3/cup8/cup8bench 1 rep PASS (cl_lg1_guest.out), host ladder 4/4 PASS (cl_lh1_host.out). Job log: job.log.
- 6fafcc6e (requested): fast suite, 21 of 30 arms run before the deadline, 21 FAIL, 0 PASS (apps_6fafcc6e_suite.out, job6.log). First failing line of arm --timer: "FAIL  RM bring-up failed at R1 openat(nvidia<gpu>): Syscall { call: "openat", errno: Some(5) }", guest RmInitAdapter failed 0x25:0x65:1249 (timer_6fafcc6e_serial_tail.log); the device log shows host-facts refusals (timer_6fafcc6e_qemu_refusals.log).
Note: the initrd/raw client was built at a6587d6a; the 6fafcc6e..a6587d6a diff touches kf-mem, kf-qemu, docs and a CI test only, not the raw-client crates (derived from git diff --stat).

Not run: app matrix (needs CUDA toolkit, bundle, guest image provisioning, 71 apps; does not fit in the time left), llm_parity, gfx_suite, video_lane, cargo tests, bare-metal fast suite.
