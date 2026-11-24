# tools/

**STATUS: LIVE, 2026-11-23.** Host-side tools, generators and research probes. None of it is linked into the product. Where a directory has its own README, that README carries its own STATUS.

| Path | What it is |
|---|---|
| `derive_*.sh`, `derive_*.py`, `hwref_spec.txt` | The generators the *Derive, never capture* rule depends on: class sets, die names, display registers and layouts, hardware reference and video method tables, produced from NVIDIA's open-gpu-kernel-modules (`third_party/ogkm-*`) by compiling its headers. Their outputs are generated Rust sources in `kf-chip`, `kf-rm`, `kf-host` and `kf-chan`. |
| `api_boundaries/` | The guest-ingress allowlist generator and its rule inputs (`docs/design/V3_GUEST_INGRESS_TABLE.md`). |
| `drivermatrix/` | The generator that sweeps ogkm release tags into version ranges and emits `crates/kf-abi/src/generated/matrix.rs` (`dm.py`, `collapse.py`, `emit_rust.py`, `regen.sh`); `scripts/drivermatrix/` is the separate hardware sweep. |
| `legal/` | The notice generator and the NVIDIA MIT licence texts it needs (`THIRD_PARTY_NOTICES.md`). |
| `cudart_probe/`, `probes/`, `util-probe/`, `read_display_caps_page.py`, `align_metadata.py` | Small measurement probes and shims. |
| `host_patches/`, `uvm_efs/` | Optional bring-up patches and UVM research; they do not establish supported UVM (`docs/howto/requirements.md`). |
| `vfio-gsp-observer/`, `windows-capture/`, `windows-debug-capture/`, `windows-guest/`, `windows-runlist-probe/`, `windows-ctrl-export/` | Capture and observation tools for the VFIO reference and the Windows driver (research; `windows-ctrl-export/` is superseded and holds only its README). |
| `display-ilut-constructor-probe/`, `display-tmo-constructor-probe/`, `memory-list-probe/` | Default-off research experiments, each with a README. |
| `tests/` | `test_align_metadata.py`, the test of `align_metadata.py`. |
