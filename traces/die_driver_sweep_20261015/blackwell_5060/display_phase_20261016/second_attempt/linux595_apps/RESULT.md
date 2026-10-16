# GB206 Linux595 CUDA and headless graphics pass

**STATUS: ANSWERED, 2026-10-16 21:31UTC. CUDA4 and headless graphics5/5+10hashes PASS.**

MEASURED only own56286918, GB206/PCI2d04/8151MiB, native595.91.07 and guest595.91.07.
Exact sourcee1bd2df4df520fbbd204b22dcb133d15bf845067/private binary SHA256
6d1fe5c4c739e8176a3baf55be40660bddf906b4d17b921d033e7a938ddb046a.
After exact native gates9/9+gate10, private build and fresh thin/client, raw30/30 at frozen180PASS
in ../cmc_probe/linux595_regression. This application lane runs afterward, strictly serial.

MEASURED595fat image installed via the public.run/open modules, verified module+libcuda595.91.07,
stock boot check15s on kernel6.8.0-142. Private installer recipe caps builds at4jobs.
The initial absent-Vulkan-loader installer warning is preserved; public Khronos/VirtualGL
provisioning follows before runtime validation. No host driver replacement or capture table.

MEASURED native CUDA4 passes first on this board. Native headless graphics S2..S5 and10hashes pass.
Then a fresh child of the stopped595parent starts privateQEMU PID118987 with actualKF3{},
displayOFF,8vCPU/8GiB/4GiBFB, actual process source/SHA/argv/driver assertions adjacent.
Guest checks: cup2 CE0xabcd1234; cup3 kernel43; cup8 N2048 bad0/maxerr0; cup8bench sizes16,2048,
20iterations/batch10, every timed result checked, BENCH_TOTAL_BAD0. All4PASS.
Guest graphics S1..S5 PASS (DRM, Vulkan compute/render, EGL, GLX via VirtualGL), all10render
hashes match bare metal exactly. 142 actual host channel births are USER.
Guest after-census:0Xid/0fliptimeouts, actual595.91.07. Host Xids10before/10after, identical lines;
those historical deliberate raw-fault Xids are not blanket-waived for application work.

MEASURED GPU VM exits0 cleanly21:29:51; safe owned receipt/name/monitor/start-time cleanup uses
ACPI/TERM with parent reap and no SIGKILL. Controller CMC_FAT595_VALIDATION_COMPLETE/EXIT0.
Only23screened text artifacts returned; images/executables/source tar stay on rental.
This is headless graphics, not a presented Cinnamon desktop. Linux595broker is UNRUN; Linux580
same-group broker still FAIL blackframe/onefliptimeout; Windows580stillCode43. No full family PASS.
Retire only own56286918 after this evidence is pushed, verify API absence, then cancel only its
verified original timer2478432. No owner GPU/VM/checkout or master changes.
