//! **H0 — the host patch tier probe** (`docs/design/V3_HOST_PATCH_LIST.md` §3.1, `V3_H5_USERD_DMA_PATCH.md`).
//!
//! A host `nvidia.ko` built with the kayfabe patch answers one extra escape, `NV_ESC_KF_QUERY`
//! (`NV_IOCTL_BASE + 40`), with `{abi, feature mask, per-feature abi}`. A stock module does not know
//! the escape and answers `EINVAL` (`nv_validate_ioctls()`: "unknown NVRM ioctl command", which also
//! writes one line to the host's kernel log; one probe per VMM start per GPU).
//!
//! Three outcomes, as the policy says: [`Probe::Present`], [`Probe::Absent`], [`Probe::Broken`].
//! `Broken` (an unexpected errno, an unexpected abi, a reply that does not parse) is treated as
//! `Absent` by every consumer and logged by name: fail closed to the stock path. **Nothing here
//! reads the driver version string to decide a feature**, and there is no `cfg` and no flag.
//!
//! ⊘ **Wired to a log line and a capability bit only.** [`HostTier::h5_dma_window`] is *not* read by
//! any behaviour yet. The behaviour it would switch (adopting the guest's USERD slot instead of the
//! USERD relay, `V3_USERD_RELAY.md`) stays off; the exact condition for turning it on is in
//! `V3_H5_USERD_DMA_PATCH.md` §7.

use kf_abi::bringup::NV_IOCTL_MAGIC;
use kf_linux_raw::{CharDevice, RawError, ioctl};

/// `NV_ESC_KF_QUERY` = `NV_IOCTL_BASE (200) + 40`
/// (`tools/host_patches/h5_userd_dma/include/nv-kf-host-patch.h`).
pub const NV_ESC_KF_QUERY: u8 = 240;
/// `NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW` = `NV_IOCTL_BASE + 41` (not issued by this crate yet).
pub const NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW: u8 = 241;
/// `sizeof(nv_ioctl_kf_query_t)`: `{flags, abi, features, reserved: u32; feature_abi: [u8; 8]}`.
pub const QUERY_SIZE: usize = 24;
/// The only query ABI this crate understands.
pub const QUERY_ABI: u32 = 1;

/// A feature a patched host can advertise. The discriminant is the bit number in the mask and the
/// index into `feature_abi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    /// H5: `NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW`, an OS-descriptor allocation whose I/O virtual
    /// addresses fit 40 bits, so that the memory can be a USERD on Volta-to-Ada.
    H5DmaWindow = 0,
}

impl Feature {
    /// Every feature this crate knows, in bit order.
    pub const ALL: [Feature; 1] = [Feature::H5DmaWindow];

    /// The ABI of the feature this crate speaks. A host that reports another number for the bit
    /// has the feature, but not in a form we can use.
    pub const fn abi(self) -> u8 {
        match self {
            Feature::H5DmaWindow => 1,
        }
    }

    /// The short name used in the tier line.
    pub const fn name(self) -> &'static str {
        match self {
            Feature::H5DmaWindow => "h5",
        }
    }
}

/// What a patched host reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Present {
    /// The query ABI (always [`QUERY_ABI`] here; anything else is `Broken`).
    pub abi: u32,
    /// The raw feature mask, bit per [`Feature`] (unknown bits are kept, never acted on).
    pub features: u32,
    /// The per-feature ABI bytes, indexed by bit number.
    pub feature_abi: [u8; 8],
}

impl Present {
    /// Is `f` advertised, enabled, and in the ABI this crate speaks?
    pub fn usable(&self, f: Feature) -> bool {
        let bit = f as u32;
        self.features & (1 << bit) != 0 && self.feature_abi[bit as usize] == f.abi()
    }
}

/// The result of one probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// The host module answered the query.
    Present(Present),
    /// A stock module (or any kernel that does not know the escape).
    Absent,
    /// The probe could not be interpreted. Treated as `Absent`; the string says why.
    Broken(String),
}

/// What the rest of the VMM may ask: which patched features are usable on this host GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTier {
    probe: Probe,
}

impl HostTier {
    /// The tier a stock host has (also the answer for any probe that is not `Present`).
    pub fn stock() -> Self {
        Self {
            probe: Probe::Absent,
        }
    }

    /// Wrap a probe result.
    pub fn from_probe(probe: Probe) -> Self {
        Self { probe }
    }

    /// The raw probe result, for evidence.
    pub fn probe(&self) -> &Probe {
        &self.probe
    }

    /// Is `f` usable on this host? `Absent` and `Broken` are never usable.
    pub fn has(&self, f: Feature) -> bool {
        matches!(&self.probe, Probe::Present(p) if p.usable(f))
    }

    /// ★ H5 capability bit: the host `nvidia.ko` can allocate an OS descriptor whose DMA addresses
    /// fit a given width. **Informational today** (log line only); see the module docs.
    pub fn h5_dma_window(&self) -> bool {
        self.has(Feature::H5DmaWindow)
    }

    /// The one status-line fragment the policy asks for (`V3_HOST_PATCH_LIST.md` §3.1):
    /// `host tier: stock` or `host tier: patched [h5]`; a broken probe says so and counts as stock.
    pub fn line(&self) -> String {
        match &self.probe {
            Probe::Absent => "host tier: stock".to_string(),
            Probe::Broken(why) => format!("host tier: stock (probe broken: {why})"),
            Probe::Present(p) => {
                let names: Vec<&str> = Feature::ALL
                    .iter()
                    .filter(|f| p.usable(**f))
                    .map(|f| f.name())
                    .collect();
                format!("host tier: patched [{}]", names.join(" "))
            }
        }
    }
}

const EINVAL: i32 = 22;
const ENOTTY: i32 = 25;

/// Interpret the outcome of the query ioctl. Pure: the unit tests drive every branch.
///
/// `Err(errno)` is the ioctl's failure; `Ok(reply)` is the buffer after a successful ioctl.
pub fn interpret(outcome: Result<[u8; QUERY_SIZE], Option<i32>>) -> Probe {
    let reply = match outcome {
        // A stock nvidia.ko: nv_validate_ioctls() -> NV_ERR_INVALID_ARGUMENT -> -EINVAL.
        // ENOTTY: a kernel layer that does not route the number at all.
        Err(Some(EINVAL)) | Err(Some(ENOTTY)) => return Probe::Absent,
        Err(Some(errno)) => return Probe::Broken(format!("errno {errno}")),
        Err(None) => return Probe::Broken("failed without an errno".to_string()),
        Ok(r) => r,
    };
    let word = |i: usize| u32::from_ne_bytes([reply[i], reply[i + 1], reply[i + 2], reply[i + 3]]);
    let (flags, abi, features, reserved) = (word(0), word(4), word(8), word(12));
    if abi != QUERY_ABI {
        return Probe::Broken(format!("query abi {abi}, expected {QUERY_ABI}"));
    }
    if flags != 0 || reserved != 0 {
        return Probe::Broken("reserved fields not zero".to_string());
    }
    let mut feature_abi = [0u8; 8];
    feature_abi.copy_from_slice(&reply[16..24]);
    Probe::Present(Present {
        abi,
        features,
        feature_abi,
    })
}

/// Build the request buffer (everything zero: `flags` must be 0).
pub const fn request() -> [u8; QUERY_SIZE] {
    [0u8; QUERY_SIZE]
}

/// Issue `NV_ESC_KF_QUERY` on an open nvidia node (`nvidiactl` or `nvidia<N>`).
///
/// Never fails: every failure is a [`Probe`]. One kernel-log line appears on a stock host.
pub fn probe(node: &CharDevice) -> Probe {
    let Ok(req) = ioctl::readwrite(NV_IOCTL_MAGIC, NV_ESC_KF_QUERY, QUERY_SIZE) else {
        return Probe::Broken("request number not buildable".to_string());
    };
    let mut arg = request();
    match node.ioctl(req, &mut arg, &mut []) {
        Ok(_) => interpret(Ok(arg)),
        Err(RawError::Syscall { errno, .. }) => interpret(Err(errno)),
        Err(other) => Probe::Broken(format!("{other:?}")),
    }
}

/// Probe and print the tier line once, by name (`kf-host: host tier: ...`).
pub fn probe_and_log(node: &CharDevice) -> HostTier {
    let tier = HostTier::from_probe(probe(node));
    eprintln!("kf-host: {}", tier.line());
    tier
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(abi: u32, features: u32, fabi: [u8; 8]) -> [u8; QUERY_SIZE] {
        let mut b = [0u8; QUERY_SIZE];
        b[4..8].copy_from_slice(&abi.to_ne_bytes());
        b[8..12].copy_from_slice(&features.to_ne_bytes());
        b[16..24].copy_from_slice(&fabi);
        b
    }

    #[test]
    fn stock_host_is_absent_on_einval_and_enotty() {
        assert_eq!(interpret(Err(Some(22))), Probe::Absent);
        assert_eq!(interpret(Err(Some(25))), Probe::Absent);
        assert_eq!(
            HostTier::from_probe(Probe::Absent).line(),
            "host tier: stock"
        );
        assert!(!HostTier::stock().h5_dma_window());
    }

    #[test]
    fn other_errno_and_no_errno_are_broken_and_count_as_stock() {
        for e in [Some(1), Some(5), Some(9), Some(14), Some(95), None] {
            let p = interpret(Err(e));
            assert!(matches!(p, Probe::Broken(_)), "{e:?}");
            let t = HostTier::from_probe(p);
            assert!(!t.h5_dma_window());
            assert!(t.line().starts_with("host tier: stock (probe broken:"));
        }
    }

    #[test]
    fn patched_host_with_h5_is_present_and_usable() {
        let p = interpret(Ok(reply(1, 1, [1, 0, 0, 0, 0, 0, 0, 0])));
        let Probe::Present(pr) = p.clone() else {
            panic!("{p:?}")
        };
        assert_eq!(pr.abi, 1);
        assert!(pr.usable(Feature::H5DmaWindow));
        let t = HostTier::from_probe(p);
        assert!(t.h5_dma_window());
        assert_eq!(t.line(), "host tier: patched [h5]");
    }

    #[test]
    fn patched_but_disabled_is_present_with_an_empty_mask() {
        // The admin set kf_dma_window=0: the module answers, the feature bit is clear.
        let t = HostTier::from_probe(interpret(Ok(reply(1, 0, [0; 8]))));
        assert!(matches!(t.probe(), Probe::Present(_)));
        assert!(!t.h5_dma_window());
        assert_eq!(t.line(), "host tier: patched []");
    }

    #[test]
    fn a_feature_in_another_abi_is_not_usable() {
        let t = HostTier::from_probe(interpret(Ok(reply(1, 1, [2, 0, 0, 0, 0, 0, 0, 0]))));
        assert!(!t.h5_dma_window());
        let t = HostTier::from_probe(interpret(Ok(reply(1, 1, [0; 8]))));
        assert!(!t.h5_dma_window());
    }

    #[test]
    fn unknown_feature_bits_are_ignored_not_acted_on() {
        let t = HostTier::from_probe(interpret(Ok(reply(1, 0b1110, [9; 8]))));
        assert!(!t.h5_dma_window());
        assert_eq!(t.line(), "host tier: patched []");
    }

    #[test]
    fn unexpected_abi_or_nonzero_reserved_is_broken() {
        assert!(matches!(
            interpret(Ok(reply(2, 1, [1; 8]))),
            Probe::Broken(_)
        ));
        assert!(matches!(
            interpret(Ok(reply(0, 1, [1; 8]))),
            Probe::Broken(_)
        ));
        let mut r = reply(1, 1, [1; 8]);
        r[12] = 1;
        assert!(matches!(interpret(Ok(r)), Probe::Broken(_)));
        let mut r = reply(1, 1, [1; 8]);
        r[0] = 1;
        assert!(matches!(interpret(Ok(r)), Probe::Broken(_)));
    }

    #[test]
    fn request_is_all_zero_and_the_escape_numbers_match_the_patch() {
        assert_eq!(request(), [0u8; QUERY_SIZE]);
        // NV_IOCTL_BASE is 200; the patch header defines the escapes as BASE+40 and BASE+41.
        assert_eq!(NV_ESC_KF_QUERY, 200 + 40);
        assert_eq!(NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW, 200 + 41);
    }

    /// The constants this module hard-codes are the ones the patch's header defines: the header
    /// text is read from the repository, so a drift in either side fails here.
    #[test]
    fn constants_equal_the_patch_header() {
        let hdr =
            include_str!("../../../tools/host_patches/h5_userd_dma/include/nv-kf-host-patch.h");
        assert!(hdr.contains("#define NV_ESC_KF_QUERY                    (NV_IOCTL_BASE + 40)"));
        assert!(hdr.contains("#define NV_ESC_KF_ALLOC_MEMORY_DMA_WINDOW  (NV_IOCTL_BASE + 41)"));
        assert!(hdr.contains("#define NV_KF_QUERY_ABI                    1"));
        assert!(hdr.contains("#define NV_KF_FEATURE_H5_DMA_WINDOW        0x1u"));
        assert!(hdr.contains("#define NV_KF_H5_DMA_WINDOW_ABI            1"));
        assert!(hdr.contains("} nv_ioctl_kf_query_t;      /* 24 bytes */"));
        assert_eq!(Feature::H5DmaWindow as u32, 0);
        assert_eq!(Feature::H5DmaWindow.abi(), 1);
        assert_eq!(QUERY_ABI, 1);
    }

    /// The thing that would be turned on is not: no consumer in this crate reads the capability
    /// bit for behaviour. (A source-level guard; the follow-up replaces it with its own test.)
    #[test]
    fn the_capability_bit_has_no_behavioural_consumer_yet() {
        let lib = include_str!("lib.rs");
        let channel = include_str!("channel.rs");
        assert!(!channel.contains("h5_dma_window"));
        assert_eq!(lib.matches("h5_dma_window").count(), 0);
    }
}
