ADAPTIVE GA102 — LIVE 2026-10-23

1. MEASURED (2026-10-23, `START.json`): lease57296386, RTX3090 candidate,$0.223333/h; auto170min armed first, deadline03:20:59Z; sourceac21aef93.
2. MEASURED (2026-10-23, `initial_inventory.txt`): pinned SSH, KVM present,23 CPUs,50367MiB RAM,175G disk/167G initially free.
3. MEASURED (2026-10-23, `native_gates/sweep_host_die.log`): RM arch0x170/impl2 confirmsGA102 on595.91.07.
4. MEASURED (2026-10-23, `native_gates/sweep_host_preflight.log`): host preflight SOUND; native driver open and device nodes usable.
5. MEASURED (2026-10-23, `native_gates/sweep_gates.log`): gates9/9+gate10PASS, USER12/12,0refused at00:51:28Z.
6. MEASURED (2026-10-23, `deployment_receipt.json`): controller1837 running, source frozenac21aef93,35-file archive/extracted allocation exporter hash verified.
7. MEASURED (2026-10-23, `parallel_transfer.log`, `appdisk_receipt.json`): pinned6.8GB app image complete140.121s/eight streams/fullSHA; NVRTC remote pin also verified.
8. MEASURED (2026-10-23, `native_gates/sweep_windows_media.log`): all four original Windows media pins pass; media helper exited0 at00:48:47Z.
9. CODE READ (2026-10-23, `harness_recipe/`): native raw/CUDA/GR baseline next, then generic Windows baseline and once-per-box calibration before bind.
10. CODE READ (2026-10-23, commitac21aef93): common Windows adapter reuses SW2 policy4ede56ac7; measured4–10x factor, ceilings, persisted stage and export reserve; side effects single attempt.
11. MEASURED (2026-10-23, `../adaptive/green.log`, `../adaptive/sw2_shared_green.log`): adaptive57/57 and shared21/21 pass; prior desktop70/70 pass.
12. MEASURED (2026-10-23, `../adaptive/mutants.json`): ten mutants killed15/6/10/6/9/2/2/3/3/2; source restored; no Rust change/added lint site, fresh clippy not run.
13. CODE READ (2026-10-23): Windows580.88 bind/ON/desktop/apps/Unigine/600s hold pending; no driver verdict is implied by earlier fixed-bound failure.
14. INFERRED (2026-10-23, `START.json`): conservative prior$6.722848 plus full new allowance$1.632778=$8.355626<12; Round3 exposure$4.552008<8; reserve$5 untouched.
15. CODE READ (2026-10-23): only own lease57296386 active; preserve bounded evidence before exact-id teardown/API absence, then Turing if a compliant offer is available.
