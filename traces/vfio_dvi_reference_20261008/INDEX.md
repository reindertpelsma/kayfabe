# INDEX — VFIO DVI-D reference, 2026-10-08

**STATUS: LIVE, 2026-10-08.** Evidence of the VFIO reference session (README.md here). Host 172.22.1.20 run directory:
`/var/lib/kf-windows-20261005/vfio-dvi-20261008/` (raw, uncompressed copies; the session disk `disk/` and `secrets/` stay
there and are not committed).

| file | what |
|---|---|
| `README.md` | the session record: setup, hypotheses with falsifiers, results (measured / inferred), comparison with kayfabe runs 88-90, host state left |
| `boot{1,2,3}-command.json` | the exact QEMU argv per boot (vfio-10's guest + patched QEMU `a8845e69`, `-msg timestamp=on -trace`) |
| `boot{1,2,3}-trace-events.txt` | the QEMU trace events enabled (boot3 adds `vfio_region_read/write`) |
| `boot{1,2,3}-qemu-trace.log.gz` | QEMU trace: every MSI/INTx delivery with a host UTC µs timestamp; boot3 also every trapped BAR0 read/write |
| `boot{1,2,3}-gsp.jsonl.gz` | x-gsp-observer capture (every GSP RPC both ways); `qpc` is host CLOCK_MONOTONIC ns |
| `boot{1,2,3}-gsp-seq.txt.gz` | the capture decoded per RPC (`vfio_kf_rpc_diff.py seq`, times in host UTC via `--mono-offset 1791227537632998000`) |
| `boot{1,2,3}-clock.txt` | QGA `guest-get-time` vs host UTC (ETW alignment) |
| `boot{1,2,3}-guest-qga.txt` | every guest script's output (D3D11 clear probe, D3D12 fence probe, HAGS/dxdiag, monitor, ETW arm/stop, user session) |
| `boot{1,2,3}-marker.txt` | start/exit lines (clean stops; host Xid count) |
| `boot2-desktop-nvidia-smi.png`, `boot3-desktop-nvidia-smi.png` | screenshots taken INSIDE the logged-on desktop: a command prompt running `nvidia-smi` |
| `boot2-dxdiag.txt` | dxdiag of the working guest (Hardware Scheduling: Enabled:True) |
| `boot{2,3}-etw.txt.gz`, `boot{2,3}-etw-histogram.txt` | boot-time DxgKrnl ETW (Base keyword), stopped live, decoded in the guest (`vdr_etw_extract.ps1`) |
| `boot3-msi-vectors-per-second.txt`, `boot3-msi-timeline.txt.gz` | each MSI's serviced interrupt vectors (`vfio_msi_vectors.py`) |
| `host-vfio-bind.txt`, `host-iommu-restore-DMA-FQ.log` | host state changes: IOMMU restore, vfio bind/unbind, lock release |
| `analysis/` | the per-question analyses (RPC diff, ETW/interrupt correlation) |

Tools (committed): `scripts/bench/windows/vfio_dvi_reference.sh` (harness), `vfio_kf_rpc_diff.py`, `vfio_msi_vectors.py`,
`d3d11_clear_probe.ps1`, `vdr_user_session.ps1`, `vdr_monitor_info.ps1`, `vdr_etw_extract.ps1`.
