# PF1 TSG parity evidence

**STATUS: RESEARCH, 2026-10-26.** Offline source/aggregate audit; no new GPU or host session. Audit base commit `6564dd5977bbcd6aa947359b55b4aa3d9a5e92f4`, WC retained inputs at commit `1eedda417`. See [full parity table](../../docs/audits/TSG_PARITY_20261026.md).

| File | Meaning |
|---|---|
| `source_manifest.json` | Pinned audit revisions, official OGKM580 commit, OGKM595 source-tar hash, 75 source-file hashes. No source snapshot or executable included. |
| `input_hashes.json` | 25 retained capture/census input paths, revisions, byte counts and hashes; each matches its pinned Git object. |
| `channel_histograms.json` | Bounded channel flag counts for run65/run64/Linux UVM, and separate first-channel-per-ctxshare/ctxshare counts for run3014. No raw address/handle payload. |
| `control_census.tsv` | 174 selected direct control rows, including explicit zero counts for selected IDs in each retained table. Parameter values remain unobserved. E/F are partial: no absence inference. |
| `short_rpc_census.tsv` | 48 selected command-function rows for 98/99/101/102/112/113/114/143 in the same retained tables. A constant in a generated matrix is not an implemented handler. |
| `native_timeslice_order.json` | Native VFIO boot3: 57 setters after first guest channel, 0 before it. **Host-birth order and parameter values unobserved.** |
| `windows_timeslice_order.json` | Bounded historical observer extracts: request/BORN order and host setter scalar-value histograms. Zero before a known birth does not license a workload-wide zero, and missing births are explicit unknowns. W53 request inventory overlaps its probe window. |
| `linux_controls.json` | Historical pre-mirror Linux CUDA simpleStreams: one 2048 µs statement, one compute-CILP setter (`flags=1,gfxp=0,cilp=2`); graphics field not selected. No effective getter. |
| `host_access.tsv` | 62 export rows: 31 selected controls in each of OGKM580.159.04/595.84. Flags/access masks identical for this selected set. Header-only GET_INTERLEAVE is not silently added as an export. |
| `ogkm_export_hashes.json` | Hashes of the ten generated NVIDIA export source files read by the metadata extraction. |
| `validation.txt` | Offline integrity, table/link/hygiene checks and claims-gate result; no new runtime test. |

MEASURED offline counts in the retained artifacts at commit `6564dd597` and commit `1eedda417` are distinct from CODE READ host behavior. Actual current-workload host readbacks, complete request payloads and UID1000 runtime outcomes are unobserved. Host access classes derive from NVIDIA export flags **and** required right masks, allocation metadata and body checks; bit0x40 is route-to-physical, not a privilege flag.

The private extraction readers were not added as product code. This evidence contains aggregate scalar flags, IDs, counts, sizes, statuses and hashes, not raw GSP messages or closed-driver disassembly. No benchmark scores or private host paths are included. No implementation constants are learned from this census.
