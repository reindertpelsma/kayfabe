# Turing thin guest `--timer` at `2939c0df` — realize passes, the guest RM stops at FWSEC

**STATUS: LIVE, 2026-09-28.** Box: vast `53080587`, GTX 1660 SUPER (TU116), host 580.159.04 open,
nested KVM. Revision on the box: `2939c0df` = `git am` of local `2e0edb06` onto `e0cb7cf8` (the same
tree; local commits are SSH-signed, so the hashes differ). kf3 binary
`/workspace/bench/kf3-bins/2939c0df/qemu-system-x86_64` (`job.log`: `KF3_BUILT … rev=2939c0df`).

| what ran | result | file |
|---|---|---|
| `build_kf3.sh`, then `KF_DEVICE=kf3 KF_ARMS=--timer run_fast_guest.sh tu3_timer 180` | **FAIL** (client rc=1 at 6.3 s). Walls 1-3 are gone: realize succeeds (`family=Turing`), the guest boots, `nvidia.ko` loads. The guest RM then fails `kgspExecuteFwsec_TU102: failed to execute FWSEC cmd 0x15: status 0x56` (×2) → the retry's scrubber path (`pBinArchive != NULL @ kernel_gsp_booter.c:487`, no Turing scrubber ucode) → `RmInitAdapter failed! (0x62:0x56:2028)` | `fast_tu3_timer_serial.log`, `fast_tu3_timer_qemu.log`, `job.log` |

Root cause (wall 4, `docs/design/V3_FAMILY_PORT_TURING.md` §2): the synthetic ROM's FWSEC descriptor
was V3 (`BOOT_FROM_HS`) for every family; Turing's `kgspExecuteHsFalcon_TU102` runs only
`BOOT_WITH_LOADER` / `BOOT_DIRECT` and answers `NV_ERR_NOT_SUPPORTED` silently for anything else.
