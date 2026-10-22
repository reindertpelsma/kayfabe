STATUS UPDATE: CLOSED PARTIAL, 2026-10-22 04:45 UTC. S3 takeover finished;
this supersedes the LIVE status below. Runtime source 7f6f7e7a7, RTX4090/AD102,
host 595.91.07 open, unchanged current-freeze Linux acceptance (each n=1;
spread N/A): native 30/30, CUDA 4/4, GR, gates 9+10, 12 USER births, guest
30/30 with 30 ordered actual-PID completions. Exact approved 5+4+1 host Xids
remain the whole journal through the final export; no later host Xid.
MEASURED: Windows installed driver 32.0.15.8088 (580.88), ON: Code 0, 1920x1080,
DWM=1, Explorer=0; zero 4101/1001 at the health check. Its two event-153 records
were Kernel-Boot, not evidence of NVIDIA TDR. Pre-hold health refused; normal
600-second hold, Windows CUDA/DX and observer arm UNRUN. No Windows PASS.
MEASURED: exact lease 57144505 absent before original timer cancellation.
Evidence: lease_57144505/windows_stopped/ and final_cleanup_receipt.json.

STATUS UPDATE: LIVE,2026-10-22. CurrentAD102LinuxvalidationCOMPLETE;
WindowsLTSCinstallactive,production580.88acceptancepending. This supersedes
priorharness-stopstatusbelow;oldmeasurementsremainhistorical.
MEASURED: sameown57144505,RTX4090/AD102,boot425c0e6f,stock595.91.07open,
frozen7f6f7e7a7;128softwarePASS,gates9+10PASS,12/12USER,native30/30,CUDA4/4,
GRPASS,guest30/30and30durableactualPIDcompletions. Exactnative5+defer4(PID5180)
+missing1(PID6626),completepositive recovery/physicaltwin correlations;frozen10,
no32/unrelated31. Source/rawproofhashesverified,all128testsremain green.
MEASURED: eachcurrent-freezehardwarearm n1,spreadN/A,noFPSclaim.
MEASURED:Windowsmediahashpinned,standardVGAinstallbootPID13736active;
latestinspectedsetup83percent. Bind/ON/desktop/CUDA/DX/normal600s+observer600s
remainUNVERIFIED;frozen10watcheractive. Evidence linux_complete/,
guest_controls_accepted/,windows_installing/ underlease_57144505/.

STATUS UPDATE: CLOSED PARTIAL, 2026-10-22 02:07 UTC; supersedes the controller
repair status below. CODE READ: explicit image directory required, preflighted
and forwarded; controller template selects its built fastguest595 directory.
MEASURED: test-only EXPECTED RED5280961c2 (6failures/0errors), fixedf98ceae65;
103testsPASS,original23green; plumbing mutants2/3/4killers,0errors.
MEASURED: newly authorized hardware rerun NOTRUN: no offer passed0.60USD/h;
no new lease. Prior AD102 measurement rows below remain the only hardware data,
n1/spreadN/A. Linux guest4+1 recovery and both Windows holds remain UNVERIFIED.
Evidence: image_path/RESULT.md, kill_table.json, availability.json.

STATUS: CLOSED PARTIAL, 2026-10-22. S2 one AD102 lease; native PASS, guest/Windows UNRUN.

MEASURED: lease57138158, RTX4090 confirmed AD102 arch0x190/impl0x2 by UID65534,
CapEff0. Runtime35ff2820e5573199072ab2d274f5f0b91a1826a8, stock595.91.07 open.

| Die / board | Cell | Result | Evidence |
|---|---|---|---|
| AD102 / RTX4090 | Native Linux595.91.07 open | MEASURED PASS n1: SOUND, gates9/9+10,12USER births,raw30/30,CUDA4/4,GR; exact native5 with recovery | lease_57138158/stopped_native_linux/ |
| AD102 / RTX4090 | Linux guest595 | MEASURED NOTRUN first --timer arm,0 guest QEMU boots; whole30-arm acceptance UNRUN | lease_57138158/linux_notrun_receipt.json |
| AD102 / RTX4090 | WindowsLTSC/actual580.88 ON | MEASURED UNRUN: bind,display,desktop,CUDA/DX,600s hold | lease_57138158/LINUX_NOTRUN_STOP.md |
| AD102 / RTX4090 | Windows580.88 observer ON | MEASURED UNRUN; observer compiled,1258 compile commands carry support flag, no observer boot | lease_57138158/pre_destroy_cleanup.txt |

CODE READ: the S2 controller adapter omitted KF_FASTGUEST_DIR; the built595 path
exists while inherited defaults do not. This is a harness error, not a measured
guest product defect. No retry, workaround, next GPU arm or second lease occurred.
MEASURED: software97/0/0 on dev and rental,original23 preserved;22mutants>=2killers.
MEASURED: native n1/spreadN/A; no guest/Windows stability or performance claim.
MEASURED: own lease destroyed/API-absent01:41:10UTC; timer cancelled afterward.
