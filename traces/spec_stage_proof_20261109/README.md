# SPECviewperf stage proof run, 2026-11-09 (first run of the stage on a real box)

**STATUS: LIVE, 2026-11-09.** MEASURED = a line in a file of this directory. Evidence is scrubbed (UUIDs, IPs, MACs, serials, URLs
replaced); no score exists in it (the stage reads status lines only).

- Box: Vast instance label `kf-spec1` (RTX 4090, AD102, host driver 595.91.07, guest driver 580.88, Windows 11 LTSC 26100), rented
  2026-11-09 21:12Z, destroyed about 23:55Z by id; about 2.3 USD (ESTIMATE: 0.852 USD/h, 2.7 h; account not queried).
- Revision: kayfabe `next` tip `734a4cc52` (Rust code and kf3 binary `kf3-bins/52a67301`, built from the first commit of the branch) plus the
  script-only commits of branch `claude/spec-stage-proof-20261109` (`0f24fecb` is the checkout the stage ran from; the QEMU binary was
  the `52a67301` one, named explicitly with `WINVM_QEMU`: the two differ only in scripts and tests).
- What ran: `windows_lane.sh` (2398 s: box 164, driver 198, tree 244, install 1669), `windows_gate.sh spec1` (`WINDOWS_GATE_PASS`,
  1560 s), the fetch helper alone with two deliberate `curl` kills, then the real stage through the real wrapper
  `scripts/ci/vast_stage_specviewperf.sh` for lane B narrowed to `medical-04` and `catia-07` with `KF_SPEC_TABLE`.
  The Worker's `/bench-data` route was replaced by a test double on the box (127.0.0.1, self-signed certificate trusted by that box only)
  that answers the helper's request with a 302 to the presigned URL; the real Worker was not involved.
- Files: `stage_3.out` (the streamed status lines, exactly what the wrapper let through), `stage.log` (the stage's own notes),
  `rows.txt`, `<viewset>.{cli,end,tdr}.txt`, `hostref_*`, `fetch_standalone.log` and `killer.log` (the resume proof),
  `fakeworker_mints.log` (Worker requests), `winlane_timing.txt`, `gate_summary.txt`, `host_log_key_lines_run2.txt` (USER_RING and
  GSP lines of the kf3 boot of the viewsets), `copy_private_status.txt`, `stage_1.out`/`stage_2.out` (the two failed attempts,
  see below), `probe_no_cli_powershell.txt` (a PowerShell failure-mode probe, not a stage run).

Results and the bugs found are in `docs/design/V3_SPEC_GATE.md`, section "First real run".
