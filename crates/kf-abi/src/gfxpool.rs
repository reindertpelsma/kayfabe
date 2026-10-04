//! GfxP pool query wire contract (OGKM 580.65.06, `ctrl2080gr.h:1298-1333`).
//!
//! The experimental encoder describes a VIRTUAL pool used only by guest-kernel
//! bookkeeping. These are not NVIDIA's physical pool dimensions. No captured
//! per-die sizes are used. Host preemption storage must remain owned by host RM.

/// `NV2080_CTRL_CMD_GR_GFX_POOL_QUERY_SIZE`.
pub const QUERY_SIZE: u32 = 0x2080_121f;
/// `sizeof(NV2080_CTRL_GR_GFX_POOL_QUERY_SIZE_PARAMS)`.
pub const QUERY_PARAMS_SIZE: usize = 40;
/// Experiment resource limit, not the SDK's 64-entry add/remove batch limit.
pub const EXPERIMENT_MAX_SLOTS: u32 = 4096;
/// A virtual slot/control block occupies one 4-KiB guest page.
const VIRTUAL_PAGE: u64 = 4096;

/// Encode the bounded virtual sizing experiment. Does not initialize or use a pool.
///
/// # Errors
/// Wrong wire size, zero slots, or more than the experiment's declared maximum.
pub fn experimental_query(params: &[u8]) -> Result<Vec<u8>, &'static str> {
    if params.len() != QUERY_PARAMS_SIZE {
        return Err("GFX_POOL_QUERY_SIZE must contain 40 bytes");
    }
    let slots = u32::from_le_bytes(params[0..4].try_into().expect("checked length"));
    if slots == 0 || slots > EXPERIMENT_MAX_SLOTS {
        return Err("GFX_POOL_QUERY_SIZE exceeds the virtual slot range 1..=4096");
    }
    let mut out = vec![0; QUERY_PARAMS_SIZE];
    out[0..4].copy_from_slice(&slots.to_le_bytes());
    out[4..8].copy_from_slice(&(VIRTUAL_PAGE as u32).to_le_bytes());
    out[8..16].copy_from_slice(&VIRTUAL_PAGE.to_le_bytes());
    out[16..24].copy_from_slice(&VIRTUAL_PAGE.to_le_bytes());
    out[24..32].copy_from_slice(&(u64::from(slots) * VIRTUAL_PAGE).to_le_bytes());
    out[32..40].copy_from_slice(&VIRTUAL_PAGE.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_lengths_and_counts_are_refused() {
        for len in [0, 4, 39, 41, 4096] {
            assert!(experimental_query(&vec![0; len]).is_err());
        }
        for slots in [0u32, EXPERIMENT_MAX_SLOTS + 1, u32::MAX] {
            let mut p = [0; QUERY_PARAMS_SIZE];
            p[..4].copy_from_slice(&slots.to_le_bytes());
            assert!(experimental_query(&p).is_err());
        }
    }

    #[test]
    fn valid_queries_replace_output_poison_and_bound_storage() {
        for slots in [1u32, 64, 128, EXPERIMENT_MAX_SLOTS] {
            let mut p = [0xff; QUERY_PARAMS_SIZE];
            p[..4].copy_from_slice(&slots.to_le_bytes());
            let out = experimental_query(&p).unwrap();
            let stride = u32::from_le_bytes(out[4..8].try_into().unwrap());
            let size = u64::from_le_bytes(out[24..32].try_into().unwrap());
            assert_eq!(size, u64::from(slots) * u64::from(stride));
            assert!(size <= 16 * 1024 * 1024);
            assert_eq!(out.len(), QUERY_PARAMS_SIZE);
            assert_eq!(&out[..4], &p[..4]);
        }
    }
}
