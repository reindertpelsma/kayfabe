// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ Every entry by which a thread starts CUDA work clears `CAP_SYS_ADMIN` first
//! (`crate::posture`; `docs/design/THE_CONSTRAINTS.md` §30).
//!
//! `src/posture.rs` tests the clear itself, and `live_cap_open_clears_before_dlopen` tests the
//! library load end to end as root. `cuInit`, `cuCtxCreate` and `cuCtxSetCurrent` cannot be
//! reached without a GPU, so this file pins in the source that each of them runs
//! `crate::posture::cuda_thread()?` before its foreign call. ⊘ Source text only: it says nothing
//! about what libcuda or RM does; the box run with the channel-alloc observer does.

fn driver_src() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/driver_unsafe.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The body of `pub fn <name>(` up to the next `pub fn`.
fn body<'a>(src: &'a str, name: &str) -> &'a str {
    let at = src
        .find(&format!("pub fn {name}("))
        .unwrap_or_else(|| panic!("★ NON-VACUITY: `pub fn {name}` is gone from driver_unsafe.rs"));
    let rest = &src[at + 1..];
    let end = rest.find("pub fn ").map_or(src.len(), |e| at + 1 + e);
    &src[at..end]
}

#[test]
fn every_cuda_entry_clears_cap_sys_admin_before_its_foreign_call() {
    let src = driver_src();
    for (entry, foreign) in [
        ("open_soname", "dlopen("),
        ("init", "(self.cuInit)"),
        ("ctx_create", "(self.cuCtxCreate)"),
        ("ctx_set_current", "(self.cuCtxSetCurrent)"),
    ] {
        let b = body(&src, entry);
        let posture = b.find("crate::posture::cuda_thread()?;").unwrap_or_else(|| {
            panic!(
                "★★★ `Cuda::{entry}` no longer calls `crate::posture::cuda_thread()?` — a thread \
                 could start CUDA work with CAP_SYS_ADMIN in effect, and libcuda's channels from \
                 it would be ADMIN"
            )
        });
        let call = b
            .find(foreign)
            .unwrap_or_else(|| panic!("★ NON-VACUITY: `{foreign}` not found in `Cuda::{entry}`"));
        assert!(
            posture < call,
            "★★★ `Cuda::{entry}` makes its foreign call before clearing CAP_SYS_ADMIN"
        );
    }
}
