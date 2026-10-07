# v3-appfix evidence (2026-09-26, vast 52689820, RTX 3060 GA106, host driver 580.159.04)

See `docs/design/V3_BUILD.md` "App-matrix fixes". kf3 binaries are keyed by revision
(`kf3-bins/<rev>`): `ce2cb06d` = master before, `b0ceaf09` = G fix + view counters,
`f372f63f` = J fix.

- `j_before_ce2cb06d_*` / `j_after_f372f63f_*`: 100 sequential CUDA processes (`um_probe malloc`)
  in ONE guest boot with persistence mode; `*_bar1.log` = host `nvidia-smi` BAR1 used every 5 s.
  Before: BAR1 256 MiB full at ~55 processes, boot hung. After: 100/100, BAR1 flat 17-19 MiB.
- `j_*views*.txt`: kf3 `views[armed released refused held]` over the boot.
- `g_*`: `nvidia-smi -l` in the guest before (exit 3, "Failed register events") / after; the nvdiff
  shim decode of host vs guest (index 37 `EVENT_SET_NOTIFICATION` answered 0x56 in the guest).
- `c_*`: managed / pageable memory shapes (`um_probe.cu`), host vs guest.
- `fix1_{host,guest}.res`: the matrix rows re-run (one app per boot in the guest).
