//! ★ 2026-10-20 MEASUREMENT ONLY -- the glue of `KF3_DIAG_USER_STALL` (stub; the implementation follows).

use super::*;

/// The name printed for a refused read.
pub(super) fn refusal_name(_why: userread::Refusal) -> &'static str {
    "?"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_reads_have_short_stable_names_and_a_store_refusal_is_vidmem() {
        use userread::Refusal as R;
        let t = [
            (R::Bounds, "bounds"),
            (R::Busy, "busy"),
            (R::Changed, "changed"),
            (R::Missing(0x1000), "unmapped"),
            (R::Topology, "topology"),
            (R::Store, "vidmem"),
            (R::Allocation, "alloc"),
            (R::Poisoned, "poisoned"),
        ];
        for (r, name) in t {
            assert_eq!(refusal_name(r), name, "{r:?}");
            assert!(name.chars().all(|c| c.is_ascii_lowercase()));
        }
        // a missing row never prints its address: the guest chose it
        assert_eq!(refusal_name(R::Missing(0xdead_beef)), refusal_name(R::Missing(1)));
    }
}
