# Host GPU identity ABI repair

**STATUS: LIVE, 2026-10-15.** GA104 / open host 595.91.07 first Windows boot
refused at realize because the GID control lacked a host-ABI inventory row.
The first failure is on `codex/sweep-linux-ampere` in `ga104_3070/windows_ga104_first_realize.log`.

Hypothesis and falsifier were stated before the fix: the public SDK control is
missing from the bounded carry table; the hypothesis fails if the correctly
carried unprivileged query still fails or yields no valid UUID.

`before.txt` records exactly one failed new test (exit 101). A focused dm.py
compiler/DWARF sweep compiled the public struct and command at every one of the
30 tags in tools/drivermatrix/tags.txt; `ranges.tsv` and `sweep.txt` record the
measurement. The layout has index/flags/length/data, sizeof 268 at every tag.
It was added to consumed.txt and the normal generated matrix. The existing
host-control ID consistency test checks the command at every measured tag.

Reproduction uses the regular sdk.spec ctrl_params/ctrl_cmds entries and
`tools/drivermatrix/regen.sh`; the focused sweep merely restricted these entries
to NV2080_CTRL_GPU_GET_GID_INFO_PARAMS and NV2080_CTRL_CMD_GPU_GET_GID_INFO.
No per-die constant, opaque layout guess, or nearest-version fallback was added.

After at f6d96614d: kf-abi, kf-rm, kf-host and kf-qemu test run succeeds. The new
test checks registration, layout, authored request carry, and size refusals at
all measured tags. See after.txt. The rental binary rebuilt at f6d96614d and the
same Windows overlay now passes realize with default auto identity. Windows
acceptance is still pending; passing realize is not a Windows pass.
