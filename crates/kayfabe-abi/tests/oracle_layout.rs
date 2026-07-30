//! Layout proof against the oracles — **not** against ourselves.
//!
//! Three independent readings of the same ABI exist in this project's trees:
//!
//! | oracle | what it is | where |
//! |---|---|---|
//! | **ogkm** | NVIDIA's own generated headers, 610.43.02 | `research_clones/ogkm/` |
//! | **nvproxy** | gVisor's independent transcription, per driver version | `gvisor/pkg/abi/nvgpu/` |
//! | **the C artifact** | this project's working implementation + its parity test | `nvidia-gpu-passthrough/src/abi/`, `tests/abi_parity/` |
//!
//! The generated code comes from the first. This file pins it against the other
//! two, with a `file:line` citation on every number, because a generator checked
//! only against its own input is a tautology.
//!
//! ★ Where the oracles **disagree**, the disagreement is written down as an
//! assertion of both sides rather than resolved silently — see
//! [`nvos46_the_three_oracles_disagree_and_here_is_exactly_how`].

use kayfabe_abi::generated::{classes, ctrl, nvos, rpc};
use kayfabe_abi::transcribed::Nvos46ParametersPre580;
use kayfabe_abi::versions::{BENCH_DRIVER, table_for};
use kayfabe_abi::wire::StructLayout;

/// A generated module's two layout tables: the generator's `STRUCTS` and the
/// `RUSTC_OFFSETS` built with `offset_of!`, named so the comparison below can be
/// written as one loop.
type ModuleTables = (
    &'static str,
    &'static [&'static StructLayout],
    &'static [(&'static str, &'static [(&'static str, usize)])],
);

/// Assert a layout's size and every field offset against a golden table.
fn assert_layout(l: &StructLayout, want_size: usize, want: &[(&str, usize)], oracle: &str) {
    assert_eq!(
        l.size, want_size,
        "{}: sizeof disagrees with {oracle}",
        l.c_name
    );
    for (name, off) in want {
        assert_eq!(
            l.offset_of(name),
            Some(*off),
            "{}: field `{name}` offset disagrees with {oracle}",
            l.c_name
        );
    }
    assert_eq!(
        l.fields.len(),
        want.len(),
        "{}: the golden table names {} fields but the layout has {} — a field was added or \
         dropped and nobody looked",
        l.c_name,
        want.len(),
        l.fields.len()
    );
}

// ---------------------------------------------------------------------------
// 1. The generator's layout vs rustc's — two algorithms, same answer.
// ---------------------------------------------------------------------------

/// Every generated struct: the offsets the generator computed equal the offsets
/// `offset_of!` reports.
///
/// The same equality is a `const` assertion inside each generated module, so a
/// mismatch already fails the build. It is ALSO a runtime test because a
/// compile-time assert is invisible to the mutation gate, and because this arm
/// asserts the *count* — a struct whose `RUSTC_OFFSETS` silently lost an entry
/// would still compile.
#[test]
fn the_generators_layout_equals_rustcs_for_every_generated_struct() {
    let modules: &[ModuleTables] = &[
        ("nvos", nvos::STRUCTS, nvos::RUSTC_OFFSETS),
        ("classes", classes::STRUCTS, classes::RUSTC_OFFSETS),
        ("ctrl", ctrl::STRUCTS, ctrl::RUSTC_OFFSETS),
        ("rpc", rpc::STRUCTS, rpc::RUSTC_OFFSETS),
    ];
    let mut checked_fields = 0usize;
    let mut checked_structs = 0usize;
    for (module, layouts, rustc) in modules {
        assert_eq!(
            layouts.len(),
            rustc.len(),
            "{module}: STRUCTS and RUSTC_OFFSETS list different numbers of structs"
        );
        for (l, (c_name, offs)) in layouts.iter().zip(rustc.iter()) {
            assert_eq!(
                l.c_name, *c_name,
                "{module}: the two tables are out of order"
            );
            assert_eq!(
                l.fields.len(),
                offs.len(),
                "{}: generator has {} fields, rustc table has {}",
                l.c_name,
                l.fields.len(),
                offs.len()
            );
            for (f, (rn, ro)) in l.fields.iter().zip(offs.iter()) {
                assert_eq!(f.rust_name, *rn, "{}: field order differs", l.c_name);
                assert_eq!(
                    f.offset, *ro,
                    "{}: field `{rn}` — generator says +{}, rustc says +{ro}",
                    l.c_name, f.offset
                );
                checked_fields += 1;
            }
            checked_structs += 1;
        }
    }
    // Non-vacuity: a green run of a loop that never iterated is a zero nobody
    // re-checks (`testing_doctrine.md` §1).
    assert_eq!(checked_structs, 14, "the slice is 14 generated structs");
    // 4+7+11+8+7+7+9 (nvos) + 4+9+5+3 (classes) + 7+7 (ctrl) + 8 (rpc). The first
    // draft of this line said 66 and the test caught it — which is the point of
    // asserting the count rather than trusting the loop ran.
    assert_eq!(checked_fields, 96, "…with 96 fields between them");
}

/// The transcribed layout gets the same treatment. A hand-written table that
/// nothing checks is a rumour.
#[test]
fn the_transcribed_layout_equals_rustcs() {
    let l = &Nvos46ParametersPre580::LAYOUT;
    let r = Nvos46ParametersPre580::RUSTC_OFFSETS;
    assert_eq!(l.fields.len(), r.len());
    assert_eq!(l.fields.len(), 9);
    for (f, (rn, ro)) in l.fields.iter().zip(r.iter()) {
        assert_eq!(f.rust_name, *rn);
        assert_eq!(
            f.offset, *ro,
            "transcribed `{rn}`: table +{}, rustc +{ro}",
            f.offset
        );
    }
    assert_eq!(l.size, core::mem::size_of::<Nvos46ParametersPre580>());
}

/// The **second** transcription — the `PROMOTE_CTX` params prefix — against rustc, and
/// then against the C artifact's own independently-written offsets.
///
/// # The C artifact as the third oracle here
///
/// `nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2441-2445` is a handler comment
/// written by a human reading the same headers, and its code
/// (`:2446-2461`) reads exactly those offsets against a real GA106 + 580 guest. So it is
/// a genuinely independent reading, not a copy of ours — and it agrees on **every offset
/// this test names**. Where it does not agree is a field WIDTH, which is
/// [`the_c_artifacts_bufferid_bug_does_not_reproduce`] below.
#[test]
fn the_promote_ctx_prefix_is_pinned_against_rustc_and_the_c_artifact() {
    use kayfabe_abi::transcribed::Nv2080CtrlGpuPromoteCtxParamsHeader as H;

    let l = &H::LAYOUT;
    let r = H::RUSTC_OFFSETS;
    assert_eq!(l.fields.len(), r.len());
    assert_eq!(l.fields.len(), 9);
    for (f, (rn, ro)) in l.fields.iter().zip(r.iter()) {
        assert_eq!(f.rust_name, *rn);
        assert_eq!(
            f.offset, *ro,
            "transcribed `{rn}`: table +{}, rustc +{ro}",
            f.offset
        );
    }
    assert_eq!(l.size, core::mem::size_of::<H>());
    assert_eq!(l.size, 48, "the prefix IS offsetof(.., promoteEntry)");

    // The C artifact's three snoop offsets, verbatim from its own comment.
    assert_eq!(
        l.offset_of("h_chan_client"),
        Some(12),
        "C: `hChanClient@+12`"
    );
    assert_eq!(l.offset_of("entry_count"), Some(40), "C: `entryCount@+40`");
    assert_eq!(H::SIZE, 48, "C: `promoteEntry[]@+48`");

    // …and the entry record it describes as "32B each: gpuPhysAddr@0, gpuVirtAddr@8,
    // size@16, physAttr@24, bufferId@28, bInitialize@30, bNonmapped@31".
    let e = &ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::LAYOUT;
    assert_eq!(e.size, 32, "C: `32B each`");
    for (name, off) in [
        ("gpu_phys_addr", 0usize),
        ("gpu_virt_addr", 8),
        ("size", 16),
        ("phys_attr", 24),
        ("buffer_id", 28),
        ("b_initialize", 30),
        ("b_nonmapped", 31),
    ] {
        assert_eq!(
            e.offset_of(name),
            Some(off),
            "C artifact says `{name}@{off}`"
        );
    }

    // The total, from the two generated numbers rather than from a literal.
    assert_eq!(
        H::PARAMS_SIZE,
        560,
        "the captured host RPC records psize=560",
    );
    assert_eq!(
        H::PARAMS_SIZE,
        H::SIZE + ctrl::NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES * e.size,
    );
}

/// ★★ **C defect D2, written as a NEGATIVE so it cannot come back.**
///
/// `C: nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2459` reads
/// `uint32_t bufferId = ldl_le_p(e + 28);` — four bytes at +28, over a `NvU16 bufferId`
/// with `bInitialize` at +30 and `bNonmapped` at +31. So its `bufferId` silently carried
/// `bufferId | (bInitialize << 16) | (bNonmapped << 24)`, and a human later
/// reverse-engineered that packing back out of the emulator's own log
/// (`mode2_cuctxcreate_resume.md:210`: *"low byte = type; `0x0001xxxx`=mapped,
/// `0x0101xxxx`=NONMAPPED"*) — an analysis artefact shaped by an ABI bug.
///
/// The width is already asserted above. This asserts the **consequence**: the two flag
/// bytes must not be able to reach `buffer_id` at all, so two entries that differ ONLY in
/// those flags must decode to the same `buffer_id`. A four-byte read fails this and a
/// two-byte read cannot.
#[test]
fn the_c_artifacts_bufferid_bug_does_not_reproduce() {
    let mut plain = [0u8; 32];
    plain[28..30].copy_from_slice(&0x00A5u16.to_le_bytes());
    // Same bufferId; both flag bytes set as loudly as they can be.
    let mut flagged = plain;
    flagged[30] = 0xFF;
    flagged[31] = 0xFF;

    let a = ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::decode(&plain).expect("decodes");
    let b = ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::decode(&flagged).expect("decodes");
    assert_eq!(a.buffer_id, 0x00A5);
    assert_eq!(
        a.buffer_id, b.buffer_id,
        "★ D2: bInitialize/bNonmapped must be unreachable from bufferId",
    );
    assert_eq!((a.b_initialize, a.b_nonmapped), (0, 0));
    assert_eq!((b.b_initialize, b.b_nonmapped), (0xFF, 0xFF));

    // And the top of the u16 is real: 0xFFFF is a bufferId, not a flag spill.
    let mut wide = [0u8; 32];
    wide[28..30].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert_eq!(
        ctrl::Nv2080CtrlGpuPromoteCtxBufferEntry::decode(&wide)
            .expect("decodes")
            .buffer_id,
        0xFFFF,
    );
}

// ---------------------------------------------------------------------------
// 2. vs gVisor nvproxy — an independent transcription by another team.
// ---------------------------------------------------------------------------

/// Field-for-field against `gvisor/pkg/abi/nvgpu/frontend.go`.
///
/// Total size alone would not catch the `nvos64_abi_fix` bug class: swapping two
/// same-width fields leaves `sizeof` identical. So every offset is named.
#[test]
fn nvos_frontend_structs_match_nvproxys_field_layout() {
    // gvisor/pkg/abi/nvgpu/frontend.go:255-262
    assert_layout(
        &nvos::Nvos00Parameters::LAYOUT,
        16,
        &[
            ("h_root", 0),
            ("h_object_parent", 4),
            ("h_object_old", 8),
            ("status", 12),
        ],
        "nvproxy frontend.go:255",
    );
    // frontend.go:300-310
    assert_layout(
        &nvos::Nvos21Parameters::LAYOUT,
        32,
        &[
            ("h_root", 0),
            ("h_object_parent", 4),
            ("h_object_new", 8),
            ("h_class", 12),
            ("p_alloc_parms", 16),
            ("params_size", 24),
            ("status", 28),
        ],
        "nvproxy frontend.go:300",
    );
    // frontend.go:371-381
    assert_layout(
        &nvos::Nvos55Parameters::LAYOUT,
        28,
        &[
            ("h_client", 0),
            ("h_parent", 4),
            ("h_object", 8),
            ("h_client_src", 12),
            ("h_object_src", 16),
            ("flags", 20),
            ("status", 24),
        ],
        "nvproxy frontend.go:371",
    );
    // frontend.go:738-748
    assert_layout(
        &nvos::Nvos54Parameters::LAYOUT,
        32,
        &[
            ("h_client", 0),
            ("h_object", 4),
            ("cmd", 8),
            ("flags", 12),
            ("params", 16),
            ("params_size", 24),
            ("status", 28),
        ],
        "nvproxy frontend.go:738",
    );
    // ★ frontend.go:788-800. `nvos64_abi_fix` was a FIELD ORDER bug in exactly
    // this struct; `pRightsRequested` before `paramsSize`, and `flags` between
    // `paramsSize` and `status`, are the three positions that were wrong.
    assert_layout(
        &nvos::Nvos64Parameters::LAYOUT,
        48,
        &[
            ("h_root", 0),
            ("h_object_parent", 4),
            ("h_object_new", 8),
            ("h_class", 12),
            ("p_alloc_parms", 16),
            ("p_rights_requested", 24),
            ("params_size", 32),
            ("flags", 36),
            ("status", 40),
        ],
        "nvproxy frontend.go:788",
    );
    // frontend.go:711-723, NVOS47_PARAMETERS_V550 — the shape from 550.54.04 on,
    // which covers every driver version this crate supports.
    assert_layout(
        &nvos::Nvos47Parameters::LAYOUT,
        48,
        &[
            ("h_client", 0),
            ("h_device", 4),
            ("h_dma", 8),
            ("h_memory", 12),
            ("flags", 16),
            ("dma_offset", 24),
            ("size", 32),
            ("status", 40),
        ],
        "nvproxy frontend.go:711 (NVOS47_PARAMETERS_V550)",
    );
    // frontend.go:654-668, NVOS46_PARAMETERS_V580.
    assert_layout(
        &nvos::Nvos46Parameters::LAYOUT,
        64,
        &[
            ("h_client", 0),
            ("h_device", 4),
            ("h_dma", 8),
            ("h_memory", 12),
            ("offset", 16),
            ("length", 24),
            ("flags", 32),
            ("flags2", 36),
            ("kind_override", 40),
            ("dma_offset", 48),
            ("status", 56),
        ],
        "nvproxy frontend.go:654 (NVOS46_PARAMETERS_V580)",
    );
    // frontend.go:625-639, the pre-580.65.06 NVOS46 — our transcription.
    assert_layout(
        &Nvos46ParametersPre580::LAYOUT,
        56,
        &[
            ("h_client", 0),
            ("h_device", 4),
            ("h_dma", 8),
            ("h_memory", 12),
            ("offset", 16),
            ("length", 24),
            ("flags", 32),
            ("dma_offset", 40),
            ("status", 48),
        ],
        "nvproxy frontend.go:625 (NVOS46_PARAMETERS)",
    );
}

/// `gvisor/pkg/abi/nvgpu/classes.go:198-211`.
#[test]
fn nv0080_alloc_params_match_nvproxys_field_layout() {
    assert_layout(
        &classes::Nv0080AllocParameters::LAYOUT,
        56,
        &[
            ("device_id", 0),
            ("h_client_share", 4),
            ("h_target_client", 8),
            ("h_target_device", 12),
            ("flags", 16),
            ("va_space_size", 24),
            ("va_start_internal", 32),
            ("va_limit_internal", 40),
            ("va_mode", 48),
        ],
        "nvproxy classes.go:198",
    );
}

// ---------------------------------------------------------------------------
// 3. vs the C artifact — a working implementation and its parity test.
// ---------------------------------------------------------------------------

/// The sizes the C artifact's `abi_parity` test asserts, for the structs where
/// the C is version-blind and therefore unambiguous.
///
/// Source: `nvidia-gpu-passthrough/tests/abi_parity/abi_parity_test.go:56-95`
/// (`TestFrontendStructSizes`) and `:118-124` (`TestAllocParamStructSizes`).
#[test]
fn sizes_match_the_c_artifacts_abi_parity_expectations() {
    assert_eq!(nvos::Nvos00Parameters::SIZE, 16, "abi_parity_test.go:58");
    assert_eq!(nvos::Nvos21Parameters::SIZE, 32, "abi_parity_test.go:56");
    assert_eq!(nvos::Nvos64Parameters::SIZE, 48, "abi_parity_test.go:57");
    assert_eq!(nvos::Nvos54Parameters::SIZE, 32, "abi_parity_test.go:59");
    assert_eq!(nvos::Nvos55Parameters::SIZE, 28, "abi_parity_test.go:60");
    assert_eq!(nvos::Nvos47Parameters::SIZE, 48, "abi_parity_test.go:78");
    assert_eq!(
        classes::Nv0080AllocParameters::SIZE,
        56,
        "abi_parity_test.go:120"
    );
}

/// ★ **A FINDING, asserted rather than resolved.**
///
/// `NVOS46_PARAMETERS` is the one struct in the slice where the three oracles do
/// not say the same thing, and the shape of the disagreement matters:
///
/// | oracle | says | citation |
/// |---|---|---|
/// | ogkm 610.43.02 | **64** (`flags2`, `kindOverride` present) | `nvos.h:2168` |
/// | nvproxy | **56** below 580.65.06, **64** from 580.65.06 | `frontend.go:625`, `:654`; `version.go:1057` |
/// | C artifact — **runtime** | 56 for ≤575, **64** for 580 | `nvkvm_abi.h:66,76,86` |
/// | C artifact — **parity test** | **56**, unconditionally | `abi_parity_test.go:68` |
///
/// nvproxy and the C's *runtime* agree, and ogkm confirms the newer form. The C's
/// *parity test* is the outlier: it asserts one size for a struct the C's own
/// runtime knows is versioned, so the test is **weaker than the code it guards**
/// and would stay green if the 580 branch of `nvkvm_abi.h` were deleted.
///
/// The bench runs 580.159.04, which is above the boundary — so the C is right at
/// runtime on its own bench and its parity test is asserting a layout the bench
/// does not use.
#[test]
fn nvos46_the_three_oracles_disagree_and_here_is_exactly_how() {
    // ogkm 610.43.02 and nvproxy's post-580.65.06 form.
    assert_eq!(nvos::Nvos46Parameters::SIZE, 64);
    assert_eq!(nvos::Nvos46Parameters::LAYOUT.offset_of("status"), Some(56));
    assert_eq!(
        nvos::Nvos46Parameters::LAYOUT.offset_of("dma_offset"),
        Some(48)
    );

    // nvproxy's pre-580.65.06 form and the C's 535/570 runtime profile — which
    // is ALSO what the C's parity test asserts, unconditionally.
    assert_eq!(Nvos46ParametersPre580::SIZE, 56);
    assert_eq!(Nvos46ParametersPre580::LAYOUT.offset_of("status"), Some(48));
    assert_eq!(
        Nvos46ParametersPre580::LAYOUT.offset_of("dma_offset"),
        Some(40)
    );

    // The delta is exactly the two added fields, and nothing before them moved —
    // which is why one version-independent view can serve both.
    for name in [
        "h_client", "h_device", "h_dma", "h_memory", "offset", "length", "flags",
    ] {
        assert_eq!(
            nvos::Nvos46Parameters::LAYOUT.offset_of(name),
            Nvos46ParametersPre580::LAYOUT.offset_of(name),
            "`{name}` must be at the same offset in both forms"
        );
    }
    assert_eq!(nvos::Nvos46Parameters::LAYOUT.offset_of("flags2"), Some(36));
    assert_eq!(
        nvos::Nvos46Parameters::LAYOUT.offset_of("kind_override"),
        Some(40)
    );
    assert_eq!(Nvos46ParametersPre580::LAYOUT.offset_of("flags2"), None);
    assert_eq!(
        Nvos46ParametersPre580::LAYOUT.offset_of("kind_override"),
        None
    );
}

/// The `SET_PAGE_DIRECTORY` prefix against the C emulator's own snoop constants.
///
/// `nvidia-gpu-passthrough/src/qemu/nvkvm_gpu_emul.c:2528-2536` reads the payload
/// at `cmd+120` and takes `physAddress` at `+0`, `numEntries` at `+8`, `flags` at
/// `+12` and `hVASpace` at `+16`. That code has booted a real guest, so these
/// four offsets are the most load-bearing numbers in the crate: they are where a
/// VAS's PDB comes from.
#[test]
fn set_page_directory_prefix_matches_the_c_emulators_snoop_offsets() {
    let l = &ctrl::Nv0080CtrlDmaSetPageDirectoryParams::LAYOUT;
    assert_eq!(
        l.offset_of("phys_address"),
        Some(0),
        "nvkvm_gpu_emul.c:2531 ldq_le_p(cmd + 120)"
    );
    assert_eq!(
        l.offset_of("num_entries"),
        Some(8),
        "nvkvm_gpu_emul.c:2528 comment"
    );
    assert_eq!(
        l.offset_of("flags"),
        Some(12),
        "nvkvm_gpu_emul.c:2532 ldl_le_p(cmd + 132)"
    );
    assert_eq!(
        l.offset_of("h_va_space"),
        Some(16),
        "nvkvm_gpu_emul.c:2533 ldl_le_p(cmd + 136)"
    );
    // The command id the C matches on: `if (fn == 76 && ldl_le_p(cmd + 88) == 0x00801813u)`
    // (`nvkvm_gpu_emul.c:2530`).
    assert_eq!(ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, 0x0080_1813);
    // The tail has ONE oracle only (ogkm 610.43.02) — asserted so that if a
    // second one ever contradicts it, this line is what changes.
    assert_eq!(l.offset_of("ch_id"), Some(20));
    assert_eq!(l.offset_of("sub_device_id"), Some(24));
    assert_eq!(l.offset_of("pasid"), Some(28));
    assert_eq!(l.size, 32);
}

/// ★ **A SECOND FINDING.** `sizeof(rpc_message_header_v03_00)` is **32**, and the
/// C emulator contradicts itself about it.
///
/// - `nvkvm_gpu_emul.c:1586` writes `rpc.length = 36` with the comment
///   `/* length = sizeof(rpc_message_header) */` — for a bare header with no
///   payload.
/// - `nvkvm_gpu_emul.c:1637` states "32-byte rpc_message_header" and does the
///   offset arithmetic `el + 48 + 32 = el + 80` for the payload base.
/// - `nvkvm_gpu_emul.c:1657` writes `rpc.length = 32 + 32` for a header plus a
///   32-byte body.
///
/// ogkm agrees with 32: seven `NvU32` plus a 4-byte union. The `36` is benign
/// today only because the message is zero-padded and both sides checksum the
/// *declared* length, so the extra four bytes are four zeros nobody reads. It is
/// still a wrong constant sitting in the GSP_INIT_DONE path.
#[test]
fn the_rpc_envelope_is_32_bytes_and_the_c_emulator_says_36_in_one_place() {
    assert_eq!(rpc::RpcMessageHeaderV0300::SIZE, 32);
    assert_eq!(
        kayfabe_abi::view::RpcEnvelope::SIZE,
        rpc::RpcMessageHeaderV0300::SIZE,
        "the view's constant and the generated struct must not drift apart"
    );
    let l = &rpc::RpcMessageHeaderV0300::LAYOUT;
    assert_eq!(l.offset_of("header_version"), Some(0));
    assert_eq!(
        l.offset_of("signature"),
        Some(4),
        "nvkvm_gpu_emul.c:1585 el+52 with el+48 = header"
    );
    assert_eq!(
        l.offset_of("length"),
        Some(8),
        "nvkvm_gpu_emul.c:1586 el+56"
    );
    assert_eq!(
        l.offset_of("function"),
        Some(12),
        "nvkvm_gpu_emul.c:1587 el+60"
    );
    assert_eq!(
        l.offset_of("rpc_result"),
        Some(16),
        "nvkvm_gpu_emul.c:1588 el+64"
    );
    assert_eq!(
        l.offset_of("rpc_result_private"),
        Some(20),
        "nvkvm_gpu_emul.c:1589 el+68"
    );
    assert_eq!(l.offset_of("sequence"), Some(24));
    assert_eq!(l.offset_of("u"), Some(28));
    assert_ne!(
        rpc::RpcMessageHeaderV0300::SIZE,
        36,
        "nvkvm_gpu_emul.c:1586 is wrong"
    );
}

/// The signature word and the two boot-path ids, against the C emulator's
/// literals.
#[test]
fn rpc_constants_match_the_c_emulators_literals() {
    assert_eq!(
        kayfabe_abi::view::RpcEnvelope::SIGNATURE_VALID,
        0x4350_5256,
        "nvkvm_gpu_emul.c:1585 stl_le_p(el + 52, 0x43505256u)"
    );
    assert_eq!(
        rpc::NV_VGPU_MSG_EVENT_GSP_INIT_DONE,
        0x1001,
        "nvkvm_gpu_emul.c:1631 posts 0x1001 as GSP_INIT_DONE"
    );
    assert_eq!(
        rpc::NV_VGPU_MSG_EVENT_POST_EVENT,
        0x1003,
        "nvkvm_gpu_emul.c:1658 posts 0x1003 as NV_VGPU_MSG_EVENT_POST_EVENT"
    );
    // rpc_global_enums.h's own numbering, spot-checked at both ends of the
    // function list so a whole-table shift would show.
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_NOP, 0);
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_ALLOC_ROOT, 2);
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_FREE, 10);
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_MAP_MEMORY_DMA, 14);
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_UNMAP_MEMORY_DMA, 15);
    assert_eq!(rpc::NV_VGPU_MSG_FUNCTION_DUP_OBJECT, 21);
    assert_eq!(
        rpc::NV_VGPU_MSG_FUNCTION_UNLOADING_GUEST_DRIVER,
        47,
        "`fn-47` by its number in docs/reference/mode2_bench_lifecycle.md"
    );
    assert_eq!(rpc::NV_VGPU_MSG_EVENT_FIRST_EVENT, 0x1000);
}

/// The NV_ESC ioctl numbers, against the C artifact's own header
/// (`nvidia-gpu-passthrough/src/abi/nvgpu.h`) via its parity test's range check.
#[test]
fn nv_esc_numbers_match_ogkm_and_stay_in_the_byte_range() {
    assert_eq!(nvos::NV_ESC_RM_FREE, 0x29);
    assert_eq!(nvos::NV_ESC_RM_CONTROL, 0x2A);
    assert_eq!(nvos::NV_ESC_RM_ALLOC, 0x2B);
    assert_eq!(nvos::NV_ESC_RM_DUP_OBJECT, 0x34);
    assert_eq!(nvos::NV_ESC_RM_MAP_MEMORY_DMA, 0x57);
    assert_eq!(nvos::NV_ESC_RM_UNMAP_MEMORY_DMA, 0x58);
    // `abi_parity_test.go:190-196` asserts every NR is in [1, 255] because the
    // ioctl encoding gives the NR one byte.
    for nr in [
        nvos::NV_ESC_RM_FREE,
        nvos::NV_ESC_RM_CONTROL,
        nvos::NV_ESC_RM_ALLOC,
        nvos::NV_ESC_RM_DUP_OBJECT,
        nvos::NV_ESC_RM_MAP_MEMORY_DMA,
        nvos::NV_ESC_RM_UNMAP_MEMORY_DMA,
    ] {
        assert!(
            (1..=0xFF).contains(&nr),
            "NR {nr:#x} outside the one-byte ioctl field"
        );
    }
    // …and they are all distinct, which the C's parity test checks only for UVM.
    let all = [
        nvos::NV_ESC_RM_FREE,
        nvos::NV_ESC_RM_CONTROL,
        nvos::NV_ESC_RM_ALLOC,
        nvos::NV_ESC_RM_DUP_OBJECT,
        nvos::NV_ESC_RM_MAP_MEMORY_DMA,
        nvos::NV_ESC_RM_UNMAP_MEMORY_DMA,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "two escapes collide on {a:#x}");
        }
    }
}

/// The class ids, against ogkm and nvproxy.
#[test]
fn class_ids_match_the_headers() {
    assert_eq!(
        classes::NV01_ROOT,
        0x0,
        "ogkm-580: cl0000.h:42 (= ogkm-610)"
    );
    assert_eq!(
        classes::NV01_ROOT_CLIENT,
        0x41,
        "ogkm-580: nvos.h:184 / ogkm-610: nvos.h:186"
    );
    assert_eq!(
        classes::NV01_DEVICE_0,
        0x80,
        "ogkm-580: cl0080.h:36 (= ogkm-610)"
    );
    // ★ B3's classes — the ones a CUDA process's subgraph is made of. Each id is
    // read off its own `cl*.h`, and each appears in `kayfabe_mocks::WireClassArch`
    // and in `DriverAbiTable::alloc_params`; a typo here would classify an object
    // as `Unknown` and declare no namespace at all, which is why they are pinned
    // as literals rather than compared to themselves.
    //
    // ★ Every citation below was read at BOTH vendored tags and is byte-identical
    // AND at the same line in each, so one line number is honest for both: the
    // `ogkm-580:` tag is written and `ogkm-610:` agrees exactly.
    assert_eq!(
        classes::FERMI_CONTEXT_SHARE_A,
        0x9067,
        "ogkm-580: cl9067.h:33 (= ogkm-610)"
    );
    assert_eq!(
        classes::FERMI_VASPACE_A,
        0x90f1,
        "ogkm-580: cl90f1.h:33 (= ogkm-610)"
    );
    assert_eq!(
        classes::KEPLER_CHANNEL_GROUP_A,
        0xa06c,
        "ogkm-580: cla06c.h:33 (= ogkm-610)"
    );
    assert_eq!(
        classes::AMPERE_CHANNEL_GPFIFO_A,
        0xc56f,
        "ogkm-580: clc56f.h:43 (= ogkm-610) — the ONE channel class; GR vs CE is \
         `engineType`, not this"
    );
    assert_eq!(
        classes::AMPERE_COMPUTE_B,
        0xc7c0,
        "ogkm-580: clc7c0.h:32 (= ogkm-610)"
    );
    assert_eq!(
        classes::AMPERE_DMA_COPY_B,
        0xc7b5,
        "ogkm-580: clc7b5.h:33 (= ogkm-610)"
    );
}

/// `NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS` and `NV_CTXSHARE_ALLOCATION_PARAMETERS`
/// — the two indirect halves of a channel's VAS resolution — have a **second
/// oracle**, and it is the 580 tree.
///
/// Neither nvproxy nor the C artifact models either struct, so 610 alone would be
/// single-oracle territory like `NV0000_ALLOC_PARAMETERS`. But
/// `research_clones/ogkm-580.159.04/src/common/sdk/nvidia/inc/nvos.h:2903-2911`
/// and `:3232-3237` declare the identical field lists in the identical order —
/// and 580.159.04 is `BENCH_DRIVER`, i.e. the guest this port is actually aimed
/// at. Agreement between the two vendored tags is what licenses the whole-struct
/// decoders (`decode_tsg_alloc_facts` / `decode_ctxshare_alloc_facts`), so it is
/// asserted here rather than asserted about.
#[test]
fn the_tsg_and_ctxshare_params_agree_between_the_two_vendored_trees() {
    let tsg = &classes::NvChannelGroupAllocationParameters::LAYOUT;
    assert_eq!(tsg.offset_of("h_object_error"), Some(0));
    assert_eq!(tsg.offset_of("h_object_ecc_error"), Some(4));
    assert_eq!(
        tsg.offset_of("h_va_space"),
        Some(8),
        "★ the only field `AllocFacts` reads"
    );
    assert_eq!(
        tsg.offset_of("engine_type"),
        Some(12),
        "declared, and dropped: `AllocFacts` has nowhere to put it"
    );
    assert_eq!(
        tsg.offset_of("b_is_calling_context_vgpu_plugin"),
        Some(16),
        "NvBool = NvU8"
    );
    assert_eq!(tsg.size, 20, "17 bytes rounded up to the 4-byte alignment");

    let cs = &classes::NvCtxshareAllocationParameters::LAYOUT;
    assert_eq!(
        cs.offset_of("h_va_space"),
        Some(0),
        "★ CtxShare declares its VASpace FIRST, unlike the TSG"
    );
    assert_eq!(cs.offset_of("flags"), Some(4));
    assert_eq!(cs.offset_of("subctx_id"), Some(8));
    assert_eq!(cs.size, 12);
}

/// ★★ **`NV_CHANNEL_ALLOC_PARAMS` is not generated, and this test is the reason.**
///
/// The two vendored trees **disagree** about it, inside the supported version
/// range and past the three fields we read:
///
/// | off | `ogkm-610` 610.43.02 | `ogkm-580` 580.159.04 |
/// |---|---|---|
/// | +20 | `flags` | `flags` |
/// | +24 | `hContextShare` | `hContextShare` |
/// | +28 | `hVASpace` | `hVASpace` |
/// | +32 | `hHandleVASpace` | `hUserdMemory[0]` |
///
/// (`ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` versus
/// `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342` — the
/// `typedef` opens at `:296` at both tags; the 610 body is one member longer.)
/// A generated 610 mirror decoded
/// against a 580 guest — and 580.159.04 IS [`BENCH_DRIVER`] — would read
/// `hUserdMemory[0]` as `hHandleVASpace` and every subsequent field one slot
/// late, including `engineType` (`+128` at 580, `+136` at 610). So the prefix
/// stops at the last offset both
/// trees spell the same way, and this test pins the number so that widening it
/// has to argue with a second oracle rather than with a comment.
#[test]
fn the_channel_alloc_prefix_stops_where_the_two_trees_stop_agreeing() {
    use kayfabe_abi::versions::CHANNEL_ALLOC_PREFIX;
    assert_eq!(
        CHANNEL_ALLOC_PREFIX, 32,
        "through hVASpace@28 inclusive, and not one byte further"
    );
    assert!(
        !classes::STRUCTS
            .iter()
            .any(|l| l.c_name == "NV_CHANNEL_ALLOC_PARAMS"),
        "★ if someone generates this struct, the version fork above becomes a silent \
         mis-decode for the bench driver — the absence is the design"
    );

    // The prefix decodes exactly the three declared fields, from a buffer built
    // here by hand at the offsets both trees agree on.
    let t = table_for(BENCH_DRIVER).expect("supported");
    let mut p = [0u8; 64];
    p[20..24].copy_from_slice(&0xF105_04A6u32.to_le_bytes()); // flags     @ +20
    p[24..28].copy_from_slice(&0xC500_0011u32.to_le_bytes()); // hCtxShare @ +24
    p[28..32].copy_from_slice(&0x7A50_0012u32.to_le_bytes()); // hVASpace  @ +28
    p[32..36].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // the divergent slot
    let f = t.decode_channel_alloc_facts(&p).expect("prefix decodes");
    assert_eq!(f.flags, 0xF105_04A6);
    assert_eq!(f.h_ctx_share, 0xC500_0011);
    assert_eq!(f.h_vaspace, 0x7A50_0012);

    // …and refuses one byte short of the prefix rather than zero-extending it.
    assert_eq!(
        t.decode_channel_alloc_facts(&p[..CHANNEL_ALLOC_PREFIX - 1]),
        Err(kayfabe_abi::wire::AbiError::Truncated {
            c_name: "NV_CHANNEL_ALLOC_PARAMS",
            need: 32,
            got: 31,
        })
    );
}

/// `NV0000_ALLOC_PARAMETERS` — the one struct in the slice with **no oracle
/// outside ogkm**, asserted as such.
///
/// Neither `gvisor/pkg/abi/nvgpu/` nor `nvidia-gpu-passthrough/src/abi/` defines
/// it (`grep -r NV0000_ALLOC_PARAMETERS` finds nothing in either tree). Inside
/// ogkm the two vendored tags now agree exactly — the full 120-byte layout is
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl0000.h:47-52` /
/// `ogkm-610: src/common/sdk/nvidia/inc/class/cl0000.h:47-52`, character for
/// character, with `NV_PROC_NAME_MAX_LENGTH = 100U` at `nvlimits.h:47` in both —
/// so the tail is no longer 610-only. What is still uncorroborated is the
/// *bottom* of the supported range: both vendored tags are ≥ 580 and the table
/// admits 550.54.04, so `pOsPidInfo` remains unverified there. The part with
/// independent corroboration is the two-field prefix, which RM's own writer
/// sets by name
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:55,70,75` /
/// `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:55,70,75` — same lines at both:
/// `root_alloc_params.hClient = hclient`, then `…processID = KERNEL_PID` or
/// `…processID = pClient->ProcID`).
///
/// That is why [`kayfabe_abi::versions::DriverAbiTable::decode_client_alloc_facts`]
/// reads 8 bytes and `alloc_param_size` reports `None` for this class.
#[test]
fn nv0000_alloc_params_has_only_its_prefix_corroborated() {
    let l = &classes::Nv0000AllocParameters::LAYOUT;
    // Corroborated by RM's own writer.
    assert_eq!(l.offset_of("h_client"), Some(0));
    assert_eq!(l.offset_of("process_id"), Some(4));
    assert_eq!(kayfabe_abi::versions::CLIENT_ALLOC_PREFIX, 8);
    // Beyond here both vendored ogkm tags agree (cl0000.h:47-52 at each) but
    // nothing outside ogkm does, and neither tag is older than 580. Pinned so an
    // oracle that disagrees changes this test rather than passing unnoticed.
    assert_eq!(
        l.offset_of("process_name"),
        Some(8),
        "char processName[NV_PROC_NAME_MAX_LENGTH]"
    );
    assert_eq!(
        l.offset_of("p_os_pid_info"),
        Some(112),
        "NvP64, 8-aligned after 100 chars"
    );
    assert_eq!(l.size, 120);
}

/// The coverage report: every struct the generator emitted is named by at least
/// one oracle assertion in this file.
///
/// `mode2_abi_agnostic_layer.md` §2.3 rule 2 — "the generated RM table IS the
/// coverage report". Enumerated-vs-exercised, made a test rather than a claim.
#[test]
fn every_generated_struct_is_covered_by_an_oracle_assertion() {
    let asserted_here = [
        "NVOS00_PARAMETERS",
        "NVOS21_PARAMETERS",
        "NVOS46_PARAMETERS",
        "NVOS47_PARAMETERS",
        "NVOS54_PARAMETERS",
        "NVOS55_PARAMETERS",
        "NVOS64_PARAMETERS",
        "NV0000_ALLOC_PARAMETERS",
        "NV0080_ALLOC_PARAMETERS",
        "NV_CHANNEL_GROUP_ALLOCATION_PARAMETERS",
        "NV_CTXSHARE_ALLOCATION_PARAMETERS",
        "NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS",
        "NV2080_CTRL_GPU_PROMOTE_CTX_BUFFER_ENTRY",
        "rpc_message_header_v03_00",
    ];
    let generated: Vec<&str> = nvos::STRUCTS
        .iter()
        .chain(classes::STRUCTS)
        .chain(ctrl::STRUCTS)
        .chain(rpc::STRUCTS)
        .map(|l| l.c_name)
        .collect();
    for g in &generated {
        assert!(
            asserted_here.contains(g),
            "generated struct `{g}` has no oracle assertion"
        );
    }
    assert_eq!(
        generated.len(),
        asserted_here.len(),
        "the coverage list and the generated set have drifted: {generated:?}"
    );
}

// ───────────────────────── the GSP element wire, keyed on version ─────────────────────────

/// ★★ **B5 — the GSP element boundary is 610, and it is now a `table_for` key.**
///
/// Before this there was **no predicate anywhere**: `ElementLayout::new` had exactly two
/// callers, both in test code, and `(570, 610]` existed only as prose in two doc comments
/// and two design docs. So no production path selected an element layout at all, and the
/// device shell had nothing to ask. The break was also stated one interval too wide —
/// r535 **and** r570 carry the same 48-byte form as 580.
///
/// The 48-byte form with `elemCount@40` is read directly at
/// `ogkm-580: src/nvidia/inc/kernel/gpu/gsp/message_queue_priv.h:43-51`; the 16-byte
/// MCTP form appears at `ogkm-610: .../message_queue_priv.h:52-67`. Those two endpoints
/// are the only ones read here — 575.64.05, 580.65.06, 580.173.02, 590.44.01, 590.48.01,
/// 595.44.02 and 595.84 are relayed evidence (`mode2_gsp_port_plan.md` §14.4) — and a
/// `>= 610` key is safe under either reading because 610 is the verified end.
#[test]
fn the_gsp_element_wire_boundary_is_610_not_570() {
    use kayfabe_abi::DriverVersion;
    use kayfabe_abi::versions::{BENCH_DRIVER, GspElementWire, GspInitArgsWire, table_for};

    let at = |major, minor, patch| {
        table_for(DriverVersion {
            major,
            minor,
            patch,
        })
        .expect("in range")
    };

    for (major, minor, patch) in [
        (550u16, 54u16, 4u16),
        (575, 64, 5),
        (580, 65, 6),
        (BENCH_DRIVER.major, BENCH_DRIVER.minor, BENCH_DRIVER.patch),
        (595, 84, 0),
        // ★ The off-by-one at the boundary, exactly as the NVOS46 test already pins for
        // 580.65.05 vs 580.65.06: one patch below 610.43.02 is still the 48-byte form.
        (609, 255, 255),
        (610, 43, 1),
    ] {
        assert_eq!(
            at(major, minor, patch).gsp_element_wire(),
            GspElementWire::Pre610,
            "{major}.{minor}.{patch} is on the 48-byte side",
        );
        assert_eq!(
            at(major, minor, patch).gsp_init_args_wire(),
            GspInitArgsWire::FourField,
            "{major}.{minor}.{patch} declares no queue geometry",
        );
    }

    for (major, minor, patch) in [(610u16, 43u16, 2u16), (999, 0, 0)] {
        assert_eq!(
            at(major, minor, patch).gsp_element_wire(),
            GspElementWire::From610_43_02,
            "{major}.{minor}.{patch} is on the 16-byte MCTP side",
        );
        assert_eq!(
            at(major, minor, patch).gsp_init_args_wire(),
            GspInitArgsWire::NineField,
        );
    }

    // The offsets each side implies, so a mis-typed table entry is caught here and not by
    // a guest that reads its checksum out of the wrong word.
    let old = GspElementWire::Pre610;
    assert_eq!(
        (old.hdr_size(), old.checksum_off(), old.seqnum_off()),
        (48, 32, 36)
    );
    assert_eq!(old.elem_count_off(), Some(40));
    assert_eq!(
        old.transport(),
        None,
        "580 carries no MCTP: ogkm-580 has no mctp_format.h"
    );

    let new = GspElementWire::From610_43_02;
    assert_eq!(
        (new.hdr_size(), new.checksum_off(), new.seqnum_off()),
        (16, 8, 12)
    );
    assert_eq!(
        new.elem_count_off(),
        None,
        "at 610 offset 40 is rpc.sequence, so writing a count there corrupts the \
         transaction id",
    );

    // Below the floor stays a refusal — the GSP key must not have introduced a fallback.
    assert!(
        table_for(DriverVersion {
            major: 535,
            minor: 0,
            patch: 0
        })
        .is_err(),
        "MISS = FAULT, still",
    );
}

/// ★★ **B6 — the 610 transport words are the driver's own, assembled from its bit fields.**
///
/// They used to be `(0x0000_0001, 0x0000_10de)` placeholders whose own doc comment admitted
/// they were invented. Re-derived here from the definitions rather than transcribed:
///
/// - `mctpCreateTransportHeader(som=1, eom=1, seid=0, deid=0, seq=0)`
///   = `REF_NUM(MCTP_HEADER_VERSION 3:0, 1) | REF_NUM(MCTP_HEADER_EOM 30:30, 1)
///     | REF_NUM(MCTP_HEADER_SOM 31:31, 1)` = `0xC000_0001`;
/// - `mctpCreateNvdmHeader(NVDM_TYPE_RM_RPC)`
///   = `REF_DEF(MCTP_MSG_HEADER_TYPE 6:0, VENDOR_PCI=0x7e)
///     | REF_DEF(MCTP_MSG_HEADER_VENDOR_ID 23:8, NV=0x10de)
///     | REF_NUM(MCTP_MSG_HEADER_NVDM_TYPE 31:24, 0x25)` = `0x2510_DE7E`
///
/// (`ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h:39-58, 79-95, 108-120`,
/// `.../nvdm_format.h:61`, used at `ogkm-610: message_queue_cpu.c:505-512`).
#[test]
fn the_610_transport_words_are_assembled_from_the_drivers_own_bit_fields() {
    use kayfabe_abi::versions::GspElementWire;

    let t = GspElementWire::From610_43_02
        .transport()
        .expect("610 carries transport words");
    assert_eq!((t.header_off, t.nvdm_off), (0, 4));

    // Assembled from the bit fields, then compared — so the constant and its derivation
    // are both in the tree and a typo in either one fails.
    let ref_num = |hi: u32, lo: u32, v: u32| (v & ((1 << (hi - lo + 1)) - 1)) << lo;
    assert_eq!(
        t.header_word,
        ref_num(3, 0, 0x1) | ref_num(30, 30, 1) | ref_num(31, 31, 1),
        "MCTP: version=1, EOM=1, SOM=1, everything else zero",
    );
    assert_eq!(t.header_word, 0xC000_0001);
    assert_eq!(
        t.nvdm_word,
        ref_num(6, 0, 0x7e) | ref_num(23, 8, 0x10de) | ref_num(31, 24, 0x25),
        "NVDM: type=VENDOR_PCI, vendor=NV, nvdmType=RM_RPC",
    );
    assert_eq!(t.nvdm_word, 0x2510_DE7E);

    // ★ And the constraint on what may ever be asserted about them: the receiver reads
    // **only** these two bit fields (`ogkm-610: message_queue_cpu.c:735-762`).
    assert_eq!(
        t.header_validated_mask, 0x0000_000F,
        "MCTP_HEADER_VERSION is 3:0 — SOM, EOM, SEQ, TAG, TO, SEID and DEID are unread",
    );
    assert_eq!(
        t.nvdm_validated_mask, 0x00FF_FF00,
        "MCTP_MSG_HEADER_VENDOR_ID is 23:8 — the message TYPE and the NVDM type byte are \
         unread",
    );
    // Non-vacuity for the masks: they must not cover the fields the driver ignores.
    assert_eq!(
        t.header_validated_mask & 0xC000_0000,
        0,
        "SOM/EOM are not validated"
    );
    assert_eq!(
        t.nvdm_validated_mask & 0xFF00_0000,
        0,
        "the NVDM type is not validated"
    );
}

/// ★ **B7 — at 580 the guest declares no queue geometry, so the table must supply it.**
///
/// `MESSAGE_QUEUE_INIT_ARGUMENTS` has four fields at 580
/// (`ogkm-580: src/nvidia/inc/kernel/gpu/gsp/gsp_init_args.h:29-34`, populated at
/// `ogkm-580: kernel_gsp.c:4486-4489`) and nine at 610
/// (`ogkm-610: gsp_init_args.h:32-45`). The five extra ones are exactly the geometry the
/// plan's §1.3 treats as negotiated — and on the bench they do not exist.
#[test]
fn the_bench_driver_declares_no_queue_geometry_and_the_table_supplies_it() {
    use kayfabe_abi::versions::{BENCH_DRIVER, GspInitArgsWire, table_for};

    let bench = table_for(BENCH_DRIVER).expect("the bench driver is supported");
    assert_eq!(bench.gsp_init_args_wire(), GspInitArgsWire::FourField);
    assert_eq!(
        bench.gsp_init_args_wire().element_hdr_size_off(),
        None,
        "★ nothing to read: on the bench the element header size is the version key's",
    );
    assert_eq!(bench.gsp_init_args_wire().min_size(), 32);
    // …and the fallback is complete, so the absence costs nothing.
    assert_eq!(bench.gsp_element_wire().hdr_size(), 48);
    assert_eq!(bench.gsp_element_size_min(), 4096);
    assert_eq!(
        bench.gsp_element_size_max(),
        65536,
        "GSP_MSG_QUEUE_ELEMENT_SIZE_MIN * 16 — and the receive staging buffer's size",
    );

    // The mirror: 610 declares it, and at a known offset.
    let new = table_for(kayfabe_abi::DriverVersion {
        major: 610,
        minor: 43,
        patch: 2,
    })
    .expect("supported");
    assert_eq!(new.gsp_init_args_wire(), GspInitArgsWire::NineField);
    assert_eq!(new.gsp_init_args_wire().element_hdr_size_off(), Some(32));
    assert_eq!(
        new.gsp_init_args_wire().min_size(),
        40,
        "32 bytes of the four common fields plus the one geometry field we read",
    );
}
