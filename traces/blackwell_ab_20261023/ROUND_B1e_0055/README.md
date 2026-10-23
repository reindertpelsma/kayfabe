# BW3 round B1e (controller 00:55, 8-core deviation), operator: Claude, 2026-10-23

STATUS: ANSWERED, 2026-10-23. Every arm is labelled "8-core deviation" (`cpu_floor_override=8`, advertised `cpu_cores_effective=8.0`, visible nproc 7). The lease was stopped by the existing host preflight gate. No arm ran.

- Per-lease override: `run_kit/operator_lease_override.py`, tests in `rehearsal/test_lease_override.py` (21 tests). RED before the module existed (`lease_override_red.txt`), GREEN after (`lease_override_green.txt`). 11 mutants, each killed by 2 or more tests (`lease_override_mutants.txt`).
  - Defaults are unchanged: CPU floor 16, 150-minute ceiling, `round_budget` constant restored after the call.
  - The override accepts exactly 8 and rejects 7. It pins offer 57427095 and allows a 3 h lease, with the cost stop still binding.
  - Labels are written by `label_evidence` through `deviation.json`.
- Denied countries for this lease: CU, IR, KP, SY, CN, HK, MO. `rehearsal/test_denied_countries.py` with MO failed 2/5 before the edit (`denied_countries_mo_red.txt`) and passed 5/5 after (`denied_countries_mo_green.txt`). Dropping MO, HK or CN is each killed by 2 tests. 69146 stays known-bad.
- MEASURED, lease 57429076:
  - Snapshot at 21:55:57Z: the pinned offer 57427095 (m99805, RTX 5060 Ti, Quebec CA, verified, 0.295 USD/h, 8.0 effective cores).
  - The pre-create watchdog (pid 2059471) was ARMED at 21:56:18.074Z. The create followed at 21:56:19.353Z, with a 3 h fixed deadline at 00:56:18Z.
  - SSH was ready at 21:57:55Z (console pin match). `df -h /` showed 167G available, and `/dev/kvm` was present (`initial_inventory.txt`). All 4 media were pinned, and the controller started at 22:01:21Z.
- MEASURED, `lease_metadata/sweep_host_preflight.log` and `lease_57429076/cpu_inventory.txt`:
  - The host driver 595.91.07 (open) loaded, and the kernel Xid list is empty.
  - The existing `host_preflight.sh` then returned `VERDICT=HOST_FAULT` ("only 7 cores", threshold 8) with EXIT=2. The controller stopped with rc=2 at 22:05:31Z, before the die probe, the gates, the native suite and Windows.
  - The VM shows 7 CPUs (lscpu: 7 sockets × 1 core). That is below the granted 8-core floor, so I did not work around it.
- MEASURED: exact-id destroy, with API absence verified at 22:12:56.778Z. Timers were cancelled only after that. The runtime estimate is 0.095 USD (0.277 h at an observed 0.343 USD/h).
- The provider retry was not used. This was a post-SSH host gate, not a pre-SSH provider failure, and the pin admits only offer 57427095.
