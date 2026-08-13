//! ★★★★★ **THE TWO DEFECTS THAT MADE THE SYSMEM ERROR NOTIFIER UNBUILDABLE — pinned so
//! neither can come back alone.**
//!
//! `[measured 2026-08-13, vh, rev aea02a52]` the guest-side fault probe could not be
//! constructed: `FAIL R33 arm 4 = the probe could not be built: Other(2147483670)`. That
//! value is `0x8000_0016`, which `rm::ioctl_error` builds as `0x8000_0000 | errno` — i.e.
//! **`errno 22`, `EINVAL`**, from `NV_ESC_RM_ALLOC_MEMORY` (`nr 39`), the one ioctl the run's
//! own census marked `errno 22`. Criterion 1 — *"the guest sees the same fault"* — had no
//! measurement at all because of it.
//!
//! There are **two** defects, one at each end of the same object, and the previous
//! investigation could not see both because it swept the flag its hypothesis named:
//!
//! | end | what refused | why |
//! |---|---|---|
//! | allocation | `EINVAL` from `NV_ESC_RM_ALLOC_MEMORY` | `_MAPPING` left at `_DEFAULT`, so RM built an mmap context around our `fd: -1` (`escape.c:341-359` → `nv-usermap.c:44-46`) |
//! | CPU map | `NV_ERR_INVALID_ARGUMENT` from `NV_ESC_RM_MAP_MEMORY` | a **sysmem** mapping is registered against the CONTROL device (`osapi.c:2266-2289`), and `nv_get_file_private(fd, ctl)` refuses any other minor (`nv.c:4102-4106`) — we passed `/dev/nvidia<N>` |
//!
//! ⊘⊘ **FIXING EITHER ONE ALONE LOOKS EXACTLY LIKE NOT FIXING IT.** Add `_NO_MAP` and the
//! refusal moves from the allocation to the map; fix the node and the allocation still
//! refuses first. That is precisely how the pair survived a deliberate two-arm sweep and got
//! written up as *"the sysmem arm may not survive natively"*. ⇒ Both halves are asserted
//! here, in one file, with the failure text naming the other half.
//!
//! ⚠ These are **source** assertions, not a run. They cannot say the driver accepts the
//! result; they say the two known-wrong shapes are not what we send. The run is the rung.

use kayfabe_isolate_host::rm::{MapNode, NotifierAperture};

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn src(rel: &str) -> String {
    let p = crate_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The body of the named `fn`, from its signature to the next item at the same indentation.
///
/// ⊘ Crude on purpose: a smarter extractor is a second thing that can be wrong, and the
/// question here is only *"does this function's text mention this token"*.
fn fn_body(source: &str, sig: &str) -> String {
    let at = source
        .find(sig)
        .unwrap_or_else(|| panic!("no `{sig}` in the source — the function was RENAMED, and a test that cannot find its subject must fail loudly rather than pass vacuously"));
    let rest = &source[at..];
    // The next `\n    /// ` or `\n    fn ` / `\n    pub fn ` at method indentation ends it.
    let end = rest[sig.len()..]
        .find("\n    /// ")
        .map_or(rest.len(), |i| i + sig.len());
    rest[..end].to_string()
}

/// ★★★ Defect 1 — the allocation must ask for `_NO_MAP`.
///
/// Without it RM dereferences the `fd` field of the request (`escape.c:347-357`), and this
/// crate's `Nvos02ParametersWithFd` has always sent `fd: -1`.
#[test]
fn the_sysmem_notifier_allocation_asks_for_no_map() {
    let s = src("src/rm.rs");
    let body = fn_body(&s, "fn alloc_notifier_mem(");
    assert!(
        body.contains("NVOS02_FLAGS_MAPPING_NO_MAP"),
        "`alloc_notifier_mem` must set `NVOS02_FLAGS_MAPPING_NO_MAP`. Without it, RmIoctl's \
         NV01_MEMORY_SYSTEM arm calls `rm_create_mmap_context(..., pApi->fd)` immediately \
         (`ogkm-580: escape.c:341-359`) and our `fd: -1` makes `nv_get_file_private` return \
         NULL (`nv-usermap.c:44-46`) => NV_ERR_INVALID_ARGUMENT => the frontend's EINVAL => \
         `Other(0x80000016)`, which is exactly what `w288nc1` measured."
    );
    assert!(
        body.contains("fd: -1"),
        "the `fd` field is still expected to be -1 — if a later rung starts filling it in, \
         the `_NO_MAP` reasoning above changes and this test must be re-derived, not deleted"
    );
}

/// ★★★ Defect 2 — the CPU map must go through the node the BACKING dictates.
#[test]
fn a_sysmem_notifier_is_mapped_through_the_control_node() {
    assert_eq!(
        MapNode::for_notifier(NotifierAperture::Sysmem),
        MapNode::Ctl,
        "a system-memory mapping is associated with the control device \
         (`ogkm-580: osapi.c:2266-2289`), and `nv_get_file_private(fd, ctl = NV_TRUE)` then \
         requires minor NV_MINOR_DEVICE_NUMBER_CONTROL_DEVICE (`nv.c:4102-4106`)"
    );
    assert_eq!(
        MapNode::for_notifier(NotifierAperture::Vidmem),
        MapNode::Gpu,
        "device-local memory lives in the GPU's own BARs, so RM keeps the per-GPU state and \
         `nv_get_file_private` requires a regular minor instead"
    );
}

/// ★★ And the two readers must actually USE it — a correct helper nobody calls is the
/// `orphan gate`'s exact shape, and it has cost this campaign three known-positives.
#[test]
fn both_notifier_readers_choose_their_node() {
    let s = src("src/rm.rs");
    for sig in [
        "pub fn read_error_notifier(",
        "fn zero_notifier(",
    ] {
        let body = fn_body(&s, sig);
        assert!(
            body.contains("MapNode::for_notifier"),
            "`{sig}` maps the notifier page and must pick its node from the aperture. A bare \
             `map_cpu` here silently means `/dev/nvidia<N>`, which RM refuses for sysmem — \
             and the refusal arrives as `Other(31)` with the ioctl census reading `failed=0`, \
             because RM reports status INSIDE the parameter struct."
        );
    }
}

/// ⊘ The negative half: nothing else in `rm.rs` may quietly map on the control node. `Ctl`
/// is right for exactly one backing, and a second call site would be a claim that needs its
/// own citation.
#[test]
fn the_control_node_is_reached_only_through_the_notifier_derivation() {
    let s = src("src/rm.rs");
    let uses = s.matches("MapNode::Ctl").count();
    assert_eq!(
        uses, 2,
        "expected exactly two USES of `MapNode::Ctl` in src/rm.rs: its arm in \
         `MapNode::for_notifier`, and the `openat(nvidiactl)` arm of \
         `map_cpu_windowed_on`. Found {uses}. A third is a NEW claim that some other object \
         is system memory, and it needs the driver citation that claim rests on."
    );
}
