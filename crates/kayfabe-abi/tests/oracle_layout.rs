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
    assert_eq!(checked_structs, 11, "the slice is 11 generated structs");
    // 4+7+11+8+7+7+9 (nvos) + 4+9 (classes) + 7 (ctrl) + 8 (rpc). The first
    // draft of this line said 66 and the test caught it — which is the point of
    // asserting the count rather than trusting the loop ran.
    assert_eq!(checked_fields, 81, "…with 81 fields between them");
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
    assert_eq!(classes::NV01_ROOT, 0x0, "ogkm cl0000.h:42");
    assert_eq!(classes::NV01_ROOT_CLIENT, 0x41, "ogkm nvos.h:185");
    assert_eq!(classes::NV01_DEVICE_0, 0x80, "ogkm cl0080.h:36");
}

/// `NV0000_ALLOC_PARAMETERS` — the one struct in the slice with a **single**
/// oracle, asserted as such.
///
/// Neither `gvisor/pkg/abi/nvgpu/` nor `nvidia-gpu-passthrough/src/abi/` defines
/// it (`grep -r NV0000_ALLOC_PARAMETERS` finds nothing in either tree). So the
/// full 120-byte layout rests on ogkm 610.43.02 alone, and the only part with
/// corroboration is the two-field prefix, which RM's own writer sets by name
/// (`ogkm src/nvidia/inc/kernel/vgpu/rpc.h:55,70,75`:
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
    // Single-oracle territory beyond here. Pinned so a second oracle that
    // disagrees changes this test rather than passing unnoticed.
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
        "NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS",
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
