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
