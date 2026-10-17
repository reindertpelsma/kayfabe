STATUS: ANSWERED, 2026-10-17. Bench parent-deadline correction CPU-verified; hardware UNRUN.

Measured314.825s initialpreparation + source360s childwait +30s capturemargin require
>=705s overall. Regressiontest fails on recorded600s caller, passes on candidate900s;
Pythoncompile PASS. Candidate refuses tolaunch if the lease cannot contain the full
program plusstopmargin. Same boundedhelper maximum900, unchanged source/QEMU/P19.

This fixes an inconsistent timeout envelope, not the NVIDIA binding problem. Actual
child PID/exitcode remainunknown and no hardware validation was rerun. Original runtime
recipe is preserved separately; this candidate was NOT executed on rental56326220.
Before any fresh GPU test, prefer the existing INF/GSP-before-first-start workflow and
native/VFIO reference; a successful childinstall still requires actual8088/class+service
GSP1/coldCode0/desktop/apps/420hold. No productchange, newconstant or WindowsPASS.

Reproduce from repository root:
python3 traces/die_driver_sweep_20261015/ga104_595_completion_20261017/installer_budget_cpu/test_installer_budget.py traces/die_driver_sweep_20261015/ga104_595_completion_20261017/installer_budget_cpu/windows_cell_budget_v2_UNRUN.py traces/die_driver_sweep_20261015/ga104_595_completion_20261017/runtime_recipes/install58088.ps1
