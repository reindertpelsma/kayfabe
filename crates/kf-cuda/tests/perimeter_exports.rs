// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **G3 — the perimeter's export census** (`v3-sec-rawaddr`, 2026-10-04; OWNER_RULINGS §R gate
//! 3: *"a reviewed table of every function the perimeter exports to safe code, each with the checks
//! it performs and the test that shows each check"*; docs/design/V3_RAWADDR_PERIMETER.md §6 G3).
//!
//! For kf-cuda the census is FULL and runs both ways: every `pub`, `pub(crate)`, `pub(super)` and
//! `pub(in …)` function in `src/**/*_unsafe.rs`, keyed `module::Type::fn` from its enclosing `mod`
//! and `impl`, must have exactly one row in [`EXPORTS`], and every row must name a function that
//! exists. A row names the checks the function performs (the design's V-numbers) and the test that
//! refuses a violating input — which must exist in this crate's sources. An export added without a
//! row, or a row whose function or test is gone, is red here.
//!
//! Outside kf-cuda the rows are PRESENCE-ONLY (not a census): the span constructors kf-linux-raw
//! exports, kf-qemu's frame hand-off, and three rows recorded OPEN so they are never read as covered.
//!
//! ⊘ What this cannot see: that a row's check is RIGHT. It pins that the function is reviewed and
//! that a named, failing-capable test exists; the review itself is the human half (§R).

/// One exported perimeter function.
struct Row {
    /// `module::Type::fn`.
    item: &'static str,
    /// The checks it performs (design §2.5), or `-` for a function that takes no input to check.
    checks: &'static str,
    /// The test that refuses a violating input (`-` when `checks` is `-`).
    test: &'static str,
}

const fn r(item: &'static str, checks: &'static str, test: &'static str) -> Row {
    Row { item, checks, test }
}

/// ★ The reviewed table. Sorted as the census prints it (file order).
const EXPORTS: &[Row] = &[
    // driver_unsafe.rs, outside `raw`
    r(
        "driver_unsafe::refused",
        "- (a constructor of a refusal)",
        "-",
    ),
    r("driver_unsafe::driver_leaks", "-", "-"),
    r("driver_unsafe::raw::leaks", "-", "-"),
    // raw — contexts
    r(
        "driver_unsafe::raw::Ctx::create",
        "NUL in the PCI id; zero devices; every driver status",
        "H4 (hardware: kf-gate2..9 bring-up)",
    ),
    r("driver_unsafe::raw::Ctx::id", "-", "-"),
    r("driver_unsafe::raw::Ctx::device_name", "-", "-"),
    r("driver_unsafe::raw::Ctx::has_graph_api", "-", "-"),
    r("driver_unsafe::raw::Ctx::make_current", "-", "-"),
    r("driver_unsafe::raw::Ctx::drain", "-", "-"),
    r("driver_unsafe::raw::Ctx::sync_calls", "-", "-"),
    // raw — V1, V1b
    r(
        "driver_unsafe::raw::sub_range",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::ctx_matches",
        "V1b",
        "an_async_operation_stays_in_one_context",
    ),
    r(
        "driver_unsafe::raw::import_len",
        "V2",
        "an_import_maps_at_least_one_byte",
    ),
    r("driver_unsafe::raw::DevRange::len", "-", "-"),
    r(
        "driver_unsafe::raw::DevRange::sub",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::DevRange::copy_async",
        "V1b, equal lengths, disjoint",
        "an_async_operation_stays_in_one_context",
    ),
    r(
        "driver_unsafe::raw::DevRange::fill_async",
        "V1b",
        "an_async_operation_stays_in_one_context",
    ),
    r(
        "driver_unsafe::raw::DevRange::copy_to_console",
        "V1b, frame context, n <= frame len (V11)",
        "an_async_operation_stays_in_one_context",
    ),
    r(
        "driver_unsafe::raw::drop_disposition",
        "F1",
        "a_failed_drain_leaks_and_never_frees",
    ),
    // raw — device memory, V2
    r(
        "driver_unsafe::raw::DevMem::alloc_zeroed",
        "len >= 1, fits usize; zero-filled over exactly len (V1)",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::DevMem::import",
        "V2: the RmExport token's own fd and length (no caller's), len >= 1, guards unwind",
        "import_takes_an_rm_export (tests/ui)",
    ),
    r("driver_unsafe::raw::DevMem::len", "-", "-"),
    r("driver_unsafe::raw::DevMem::ctx_id", "-", "-"),
    r(
        "driver_unsafe::raw::DevMem::share",
        "- (refcount; never exposed to safe code)",
        "-",
    ),
    r(
        "driver_unsafe::raw::DevMem::range",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::DevMem::whole",
        "- (never empty by construction)",
        "-",
    ),
    r(
        "driver_unsafe::raw::DevMem::write",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::DevMem::read",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "driver_unsafe::raw::DevMem::fill",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    // raw — streams, events
    r("driver_unsafe::raw::Stream::create", "-", "-"),
    r(
        "driver_unsafe::raw::Stream::host_signal",
        "F1 (keeps the fd open until a drained drop)",
        "a_failed_drain_leaks_and_never_frees",
    ),
    r("driver_unsafe::raw::Stream::begin_capture", "-", "-"),
    r(
        "driver_unsafe::raw::Stream::end_capture",
        "- (no input; ends the capture on every path)",
        "-",
    ),
    r("driver_unsafe::raw::Event::create", "-", "-"),
    r(
        "driver_unsafe::raw::Event::record",
        "event and stream share a context",
        "an_async_operation_stays_in_one_context",
    ),
    r(
        "driver_unsafe::raw::Event::query",
        "mints Drained only on completion, naming the flight its record was stamped with",
        "a_completion_proof_of_an_earlier_flight_is_refused",
    ),
    r(
        "driver_unsafe::raw::Event::record_flight",
        "the stage's context; stamps the flight (a graph re-stamps it per launch)",
        "a_completion_proof_of_an_earlier_flight_is_refused",
    ),
    r(
        "driver_unsafe::raw::proof_covers",
        "V4: a proof covers the flight in progress",
        "a_completion_proof_of_an_earlier_flight_is_refused",
    ),
    r("driver_unsafe::raw::Event::elapsed_us", "-", "-"),
    // raw — modules, kernels, V3
    r(
        "driver_unsafe::raw::Module::load",
        "committed PTX only (an enum, never bytes)",
        "the_kernel_table_is_the_ptx",
    ),
    r(
        "driver_unsafe::raw::Module::kernel",
        "only names KERNEL_SIGS pins",
        "the_kernel_table_is_the_ptx",
    ),
    r(
        "driver_unsafe::raw::check_args",
        "V3",
        "a_launch_is_refused_unless_every_argument_matches_the_pinned_signature",
    ),
    r(
        "driver_unsafe::raw::Kernel::launch",
        "V3, V4 (puts the stage in flight)",
        "a_launch_is_refused_unless_every_argument_matches_the_pinned_signature",
    ),
    r(
        "driver_unsafe::raw::Kernel::probe_oversized_block",
        "V3 with only the block limit waived",
        "a_launch_is_refused_unless_every_argument_matches_the_pinned_signature",
    ),
    r(
        "driver_unsafe::raw::GraphExec::upload",
        "stream context (hardware)",
        "H4 (hardware: kf-gate8 graph submission)",
    ),
    r("driver_unsafe::raw::GraphExec::updates", "-", "-"),
    r(
        "driver_unsafe::raw::GraphExec::launch",
        "V3 per rewritten node, V4; re-stamps the flight events",
        "a_launch_is_refused_unless_every_argument_matches_the_pinned_signature",
    ),
    r(
        "driver_unsafe::raw::node_needs_rewrite",
        "- (a cost filter: which captured nodes a stale launch re-sets)",
        "-",
    ),
    // raw — the pinned stage, V4
    r(
        "driver_unsafe::raw::Region::host_writable",
        "V4",
        "the_stage_layout_is_aligned_disjoint_and_bounded",
    ),
    r(
        "driver_unsafe::raw::stage_layout",
        "V4",
        "the_stage_layout_is_aligned_disjoint_and_bounded",
    ),
    r(
        "driver_unsafe::raw::Flight::step",
        "V4, F2",
        "the_stage_belongs_to_the_gpu_until_a_proof_returns_it",
    ),
    r(
        "driver_unsafe::raw::PinnedStage::new",
        "V4 layout",
        "the_stage_layout_is_aligned_disjoint_and_bounded",
    ),
    r("driver_unsafe::raw::PinnedStage::region_len", "-", "-"),
    r(
        "driver_unsafe::raw::PinnedStage::write",
        "V4: host-writable, region bounds, Idle",
        "the_stage_belongs_to_the_gpu_until_a_proof_returns_it",
    ),
    r(
        "driver_unsafe::raw::PinnedStage::read",
        "V4: region bounds, Idle",
        "the_stage_belongs_to_the_gpu_until_a_proof_returns_it",
    ),
    r(
        "driver_unsafe::raw::PinnedStage::end_flight",
        "V4: a proof of THIS context and of the flight in progress",
        "a_completion_proof_of_an_earlier_flight_is_refused",
    ),
    r(
        "driver_unsafe::raw::PinnedStage::poison",
        "F2",
        "the_stage_belongs_to_the_gpu_until_a_proof_returns_it",
    ),
    r("driver_unsafe::raw::PinnedStage::flight", "-", "-"),
    r(
        "driver_unsafe::raw::encode_kf_args",
        "every range of one context; layout pinned to the .cu",
        "the_kfargs_layout_is_the_compilers",
    ),
    r("driver_unsafe::raw::ConsoleDst::span", "-", "-"),
    r("driver_unsafe::raw::ConsoleDst::ctx_id", "-", "-"),
    r(
        "driver_unsafe::raw::console_pages",
        "V12 mechanism: owned mapping, register, then leak",
        "only_owned_private_anonymous_memory_becomes_a_static_span (kf-linux-raw)",
    ),
    r("driver_unsafe::raw::CompletionFd::new", "-", "-"),
    r("driver_unsafe::raw::CompletionFd::signal", "-", "-"),
    r("driver_unsafe::raw::CompletionFd::drain", "-", "-"),
    r("driver_unsafe::raw::CompletionFd::wait_readable", "-", "-"),
    // walk_gpu_unsafe.rs — V5..V8
    r(
        "walk_gpu_unsafe::kf_format_check",
        "V5",
        "the_format_check_refuses_every_arm_of_the_cus",
    ),
    r(
        "walk_gpu_unsafe::validate_cfg",
        "V5",
        "the_configuration_and_the_requirement_table_are_bounded",
    ),
    r(
        "walk_gpu_unsafe::requirements",
        "V5",
        "the_configuration_and_the_requirement_table_are_bounded",
    ),
    r(
        "walk_gpu_unsafe::check_requirements",
        "V5",
        "the_configuration_and_the_requirement_table_are_bounded",
    ),
    r(
        "walk_gpu_unsafe::validate_stage",
        "V6",
        "a_stage_is_refused_past_any_pool_on_any_row",
    ),
    r(
        "walk_gpu_unsafe::validate_move",
        "V7",
        "a_slot_move_stays_in_the_pool_and_never_overlaps",
    ),
    r("walk_gpu_unsafe::DeviceImage::len", "-", "-"),
    r("walk_gpu_unsafe::DeviceImage::is_empty", "-", "-"),
    r(
        "walk_gpu_unsafe::WalkGpu::bring_up",
        "V5",
        "the_format_check_refuses_every_arm_of_the_cus",
    ),
    r("walk_gpu_unsafe::WalkGpu::make_current", "-", "-"),
    r(
        "walk_gpu_unsafe::WalkGpu::import_store",
        "V2 (an RmExport only); a second import refused",
        "import_takes_an_rm_export (tests/ui)",
    ),
    r("walk_gpu_unsafe::WalkGpu::store_len", "-", "-"),
    r(
        "walk_gpu_unsafe::WalkGpu::store_write",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::store_read",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::upload",
        "non-empty; V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::image_read",
        "context; V1",
        "a_stage_is_refused_past_any_pool_on_any_row",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::image_write",
        "context; V1",
        "a_stage_is_refused_past_any_pool_on_any_row",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::launch",
        "V6, V4, V3",
        "a_stage_is_refused_past_any_pool_on_any_row",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::poll",
        "V4, F2",
        "the_stage_belongs_to_the_gpu_until_a_proof_returns_it",
    ),
    r("walk_gpu_unsafe::WalkGpu::gpu_done_now", "-", "-"),
    r(
        "walk_gpu_unsafe::WalkGpu::read_report",
        "V4 (Flight::step), V8 clamps",
        "a_read_back_stays_inside_what_was_allocated",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::move_slot",
        "V7, Idle, V1, V1b",
        "a_slot_move_stays_in_the_pool_and_never_overlaps",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::read_walk_region",
        "V8, Idle, V1",
        "a_read_back_stays_inside_what_was_allocated",
    ),
    r(
        "walk_gpu_unsafe::WalkGpu::probe_oversized_block",
        "Idle; V3 (block limit waived)",
        "a_launch_is_refused_unless_every_argument_matches_the_pinned_signature",
    ),
    r("walk_gpu_unsafe::WalkGpu::completion_fd", "-", "-"),
    r("walk_gpu_unsafe::WalkGpu::ctx_sync_calls", "-", "-"),
    r("walk_gpu_unsafe::WalkGpu::submits_as_graph", "-", "-"),
    r("walk_gpu_unsafe::WalkGpu::graph_param_updates", "-", "-"),
    r("walk_gpu_unsafe::WalkGpu::pool_bytes", "-", "-"),
    // display_gpu_unsafe.rs — V9..V12
    r("display_gpu_unsafe::ConsoleFrame::len", "-", "-"),
    r("display_gpu_unsafe::ConsoleFrame::is_empty", "-", "-"),
    r("display_gpu_unsafe::ConsoleFrame::span", "- (opaque)", "-"),
    r(
        "display_gpu_unsafe::ConsoleFrame::read",
        "bounds before allocating (frame_read_fits), then MappedRegion::read_into; content racy by design",
        "a_frame_read_is_bounded_before_it_allocates",
    ),
    r(
        "display_gpu_unsafe::frame_read_fits",
        "off + n inside the frame, overflow first",
        "a_frame_read_is_bounded_before_it_allocates",
    ),
    r(
        "display_gpu_unsafe::compose_layer_fits",
        "V10",
        "a_compose_launch_is_bounded_before_it_is_queued",
    ),
    r(
        "display_gpu_unsafe::composition_bytes",
        "V9",
        "a_composition_is_bounded_on_both_sides",
    ),
    r(
        "display_gpu_unsafe::console_frame_bytes",
        "V12, F3",
        "console_frames_are_capped_in_count_and_size",
    ),
    r("display_gpu_unsafe::DisplayGpu::bring_up_on", "-", "-"),
    r("display_gpu_unsafe::DisplayGpu::make_current", "-", "-"),
    r("display_gpu_unsafe::DisplayGpu::completion_fd", "-", "-"),
    r(
        "display_gpu_unsafe::DisplayGpu::import_store",
        "V2 (an RmExport only); a second import refused",
        "import_takes_an_rm_export (tests/ui)",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::read_store",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::write_store",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::zero_store",
        "V1",
        "a_range_is_inside_its_allocation_or_refused",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::console_frame",
        "V12, F3",
        "console_frames_are_capped_in_count_and_size",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::console_frames_minted",
        "-",
        "-",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::compose_begin",
        "V9",
        "a_composition_is_bounded_on_both_sides",
    ),
    r(
        "display_gpu_unsafe::DisplayGpu::selftest_compose",
        "V10, extent <= surface",
        "a_compose_launch_is_bounded_before_it_is_queued",
    ),
    r(
        "display_gpu_unsafe::Composer::layer",
        "V10; extent inside the store (V1)",
        "the_compose_address_function_stays_inside_every_accepted_layer",
    ),
    r(
        "display_gpu_unsafe::Composer::finish",
        "V11 (the frame type: tests/ui finish_needs_a_console_frame)",
        "a_composition_finishes_only_into_a_frame_that_holds_it",
    ),
];

/// ★ Presence-only rows outside kf-cuda: `(file, the declaration that must exist, status)`.
const ELSEWHERE: &[(&str, &str, &str)] = &[
    (
        "crates/kf-linux-raw/src/mapping_unsafe.rs",
        "pub fn static_span(&'static self)",
        "V15",
    ),
    (
        "crates/kf-linux-raw/src/mapping_unsafe.rs",
        "pub fn host_span(&self) -> HostSpan",
        "through HostSpan::within",
    ),
    (
        "crates/kf-linux-raw/src/mapping_unsafe.rs",
        "pub struct StaticSpan",
        "opaque; no Hash/Ord/PartialEq",
    ),
    (
        "crates/kf-qemu/src/ffi_unsafe.rs",
        // The FFI entry point; the needle omits the qualifier keyword so this file stays out of
        // the unsafe-surface gate's file list.
        "extern \"C\" fn kf3_display_frame(",
        "V13",
    ),
    (
        "crates/kf-qemu/src/ffi_unsafe.rs",
        "pub fn check_frame(",
        "V13",
    ),
    (
        "crates/kf-qemu/src/ffi_unsafe.rs",
        "pub fn check_frame_geometry(",
        "V13 (= kf3.h kf3_frame_ok, V14)",
    ),
    // ⊘ OPEN — listed so they are never read as covered (design §10).
    (
        "crates/kf-linux-raw/src/chardev_unsafe.rs",
        "pub fn ioctl",
        "OPEN: S1-40, companion size fields",
    ),
    (
        "crates/kf-linux-raw/src/mapping_unsafe.rs",
        "pub(crate) fn addr_at",
        "OPEN: a u64 address to kf-linux-raw's safe files",
    ),
    (
        "crates/kf-linux-raw/src/window_unsafe.rs",
        "fn userspace_addr_at",
        "OPEN: a u64 address to kf-linux-raw's safe files",
    ),
];

/// Code with comments, string and char literals blanked (lines kept).
fn strip(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let (mut out, mut i) = (String::new(), 0);
    while i < b.len() {
        let c = b[i] as char;
        let nx = b.get(i + 1).copied().unwrap_or(0) as char;
        if c == '/' && nx == '/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && nx == '*' {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == b'\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
        } else if c == 'r'
            && (nx == '#' || nx == '"')
            && (i == 0 || !(b[i - 1] as char).is_alphanumeric())
        {
            let mut j = i + 1;
            while j < b.len() && b[j] == b'#' {
                j += 1;
            }
            if b.get(j) != Some(&b'"') {
                out.push(c);
                i += 1;
                continue;
            }
            let hashes = j - i - 1;
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let mut k = j + 1;
            while k < b.len() && !b[k..].starts_with(&close) {
                if b[k] == b'\n' {
                    out.push('\n');
                }
                k += 1;
            }
            out.push_str("\"\"");
            i = k + close.len();
        } else if c == '"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                } else if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            out.push_str("\"\"");
            i += 1;
        } else if c == '\'' {
            // a char literal closes within a few bytes; a lifetime does not
            let lit = (2..=10).find(|&k| {
                b.get(i + k) == Some(&b'\'')
                    && (b[i + 1] == b'\\' || k == 2 || !b[i + 1..i + k].is_ascii())
            });
            match lit {
                Some(k) => {
                    out.push_str("' '");
                    i += k + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.lines().map(str::to_string).collect()
}

/// ★ The census of one perimeter file: every non-private `fn`, keyed `module::Type::fn`.
fn census(src: &str, module: &str) -> Vec<String> {
    let mut stack: Vec<(String, i64)> = Vec::new();
    let (mut depth, mut out) = (0i64, Vec::new());
    for line in strip(src) {
        let s = line.trim();
        let words: Vec<&str> = s.split_whitespace().collect();
        let is_pub = s.starts_with("pub ") || s.starts_with("pub(");
        let fn_name = words
            .iter()
            .position(|w| *w == "fn")
            .and_then(|at| words.get(at + 1));
        if let (true, Some(name)) = (is_pub, fn_name) {
            let name = name.split(['(', '<']).next().unwrap_or("");
            let mut key = vec![module.to_string()];
            key.extend(stack.iter().map(|(n, _)| n.clone()));
            key.push(name.to_string());
            out.push(key.join("::"));
        }
        let opens = line.matches('{').count() as i64;
        let closes = line.matches('}').count() as i64;
        let head = s
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ");
        if let Some(rest) = head.strip_prefix("mod ") {
            if s.ends_with('{') {
                stack.push((rest.trim_end_matches(['{', ' ']).to_string(), depth));
            }
        } else if (s.starts_with("impl ") || s.starts_with("impl<")) && s.ends_with('{') {
            let ty = s.trim_end_matches('{').trim();
            let ty = ty.rsplit(" for ").next().unwrap_or(ty);
            let ty = ty.split_whitespace().last().unwrap_or("");
            let ty = ty
                .split('<')
                .next()
                .unwrap_or("")
                .rsplit("::")
                .next()
                .unwrap_or("");
            stack.push((ty.to_string(), depth));
        }
        depth += opens - closes;
        while stack.last().is_some_and(|(_, d)| depth <= *d) {
            stack.pop();
        }
    }
    out
}

fn src(rel: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The whole perimeter census of kf-cuda.
fn kf_cuda_census() -> Vec<String> {
    let mut all = Vec::new();
    for (file, module) in [
        ("src/driver_unsafe.rs", "driver_unsafe"),
        ("src/driver_unsafe/walk_gpu_unsafe.rs", "walk_gpu_unsafe"),
        (
            "src/driver_unsafe/display_gpu_unsafe.rs",
            "display_gpu_unsafe",
        ),
        ("src/driver_unsafe/tests_unsafe.rs", "tests_unsafe"),
    ] {
        all.extend(census(&src(file), module));
    }
    all
}

/// Every `*_unsafe.rs` under src/ is one of the four censused (a new perimeter file needs a census).
#[test]
fn every_perimeter_file_is_censused() {
    let mut found = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with("_unsafe.rs") {
                found.push(p.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    assert_eq!(
        found,
        [
            "display_gpu_unsafe.rs",
            "driver_unsafe.rs",
            "tests_unsafe.rs",
            "walk_gpu_unsafe.rs"
        ]
    );
}

/// ★★★ The census and the table agree, both ways; every named test exists.
#[test]
fn every_exported_perimeter_function_has_a_reviewed_row() {
    let census = kf_cuda_census();
    let rows: Vec<&str> = EXPORTS.iter().map(|r| r.item).collect();
    let missing: Vec<&String> = census
        .iter()
        .filter(|c| !rows.contains(&c.as_str()))
        .collect();
    let stale: Vec<&&str> = rows
        .iter()
        .filter(|r| !census.iter().any(|c| c == **r))
        .collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "★ PERIMETER EXPORT CENSUS (OWNER_RULINGS §R gate 3): exported without a reviewed row: \
         {missing:?}; rows naming no function: {stale:?}"
    );
    assert_eq!(
        census.len(),
        EXPORTS.len(),
        "one row per export: {census:?}"
    );
    let corpus: String = [
        "src/driver_unsafe.rs",
        "src/driver_unsafe/walk_gpu_unsafe.rs",
        "src/driver_unsafe/display_gpu_unsafe.rs",
        "src/driver_unsafe/tests_unsafe.rs",
        "tests/walk_abi_matches_the_cu.rs",
        "../kf-linux-raw/src/mapping_unsafe.rs",
    ]
    .iter()
    .map(|f| src(f))
    .collect();
    for row in EXPORTS {
        assert!(
            !row.checks.is_empty(),
            "{}: say what it checks, or `-`",
            row.item
        );
        if row.test == "-" {
            assert!(
                row.checks.starts_with('-'),
                "{}: a check with no test that shows it",
                row.item
            );
            continue;
        }
        if row.test.starts_with("H4 ") {
            // a check only a GPU can exercise: the merge bar's hardware row (design §8 H4)
            continue;
        }
        let name = row.test.split_whitespace().next().unwrap();
        let in_ui = row.test.contains("(tests/ui)")
            && std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/ui/{name}.rs"))
                .exists();
        assert!(
            in_ui || corpus.contains(&format!("fn {name}(")),
            "{}: its test `{name}` does not exist",
            row.item
        );
    }
}

/// The presence-only rows outside kf-cuda — including the OPEN ones.
#[test]
fn the_rows_outside_kf_cuda_name_existing_declarations() {
    for (file, decl, status) in ELSEWHERE {
        let text = src(&format!("../../{file}"));
        assert!(
            text.contains(decl),
            "{file}: `{decl}` ({status}) is gone — update the row"
        );
    }
}

/// ⊘ The census must be able to FAIL: an unlisted `pub fn`, in an `impl` inside a `mod`, behind a
/// string holding a brace and a comment holding a `fn`, is found and keyed exactly.
#[test]
fn the_census_finds_an_unlisted_export() {
    let fixture = r#"
mod raw {
    // pub fn not_this_one() {
    pub struct T;
    impl T {
        pub(in crate::driver_unsafe) fn hidden(&self) -> &str { "{ not a block" }
        fn private(&self) {}
    }
    pub(super) fn free<'a>(x: &'a u8) -> char { let _ = x; '{' }
}
pub fn top() {}
"#;
    let c = census(fixture, "driver_unsafe");
    assert_eq!(
        c,
        [
            "driver_unsafe::raw::T::hidden",
            "driver_unsafe::raw::free",
            "driver_unsafe::top"
        ]
    );
    let rows: Vec<&str> = EXPORTS.iter().map(|r| r.item).collect();
    assert!(
        c.iter().any(|x| !rows.contains(&x.as_str())),
        "the comparison reports it"
    );
}
