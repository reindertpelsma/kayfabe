STATUS: SUPERSEDED, 2026-10-31. This directory held `derive.py` and `stubs.txt`, which generated
`crates/kf-abi/src/hoststub_generated.rs` from public OGKM and the pinned retail Windows driver.
Both files are deleted: the generator read two values (the control's export flags and the export
row's position) out of a closed binary, and the owner's rule is that nothing derived from a closed
binary stands in the public tree (`docs/design/V3_PROVENANCE.md` section 2).

The stub table is now hand-maintained in `crates/kf-abi/src/hoststub.rs`, one source-tagged row per
item: the interface value is OGKM 580.65.06 (`g_finn_rm_api.h:425`), the message number `0xd` and
the declared params size `0` are measured from committed traces. The flags and the row position
are not carried at all (nothing branched on them; they were only logged).
`crates/kf-abi/tests/windows_cell_provenance.rs` refuses any row without an accepted source tag and
re-reads every cited trace line.
