//! ★★★ **The classes WE allocate on the host — derived, never hand-picked.**
//!
//! The old tree kept one hand-written profile per family (`Ga10xHostClasses`, …) naming one id per
//! kind, and they were wrong exactly where a family spans die groups: GA100 lists
//! `AMPERE_COMPUTE_A`/`AMPERE_DMA_COPY_A`, GA10x the `_B` classes; GB202 lists `BLACKWELL_*_B` where
//! GB100 lists `_A` (`classes.rs` header). ⇒ v3 asks the HOST: `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2`
//! (`0x800292`, NON_PRIVILEGED — `g_device_nvoc.c:333` flags `0x1010b`) returns the classes this die
//! supports, and the choice per kind is the NEWEST id in *family set ∩ host list*.
//!
//! ⊘ Zero per-die maintenance: a new die of a known family is covered by the family's generated set
//! and its own class list. A kind the host does not list is refused by name — never a guessed id.

use crate::classes::{Kind, classes_for};
use crate::Family;
use kf_arch::ids::ClassId;
use kf_arch::{CeObjectClass, ChannelClass, ComputeObjectClass, HostClasses, UsermodeClass};

/// `NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2` (`ogkm-580: ctrl0080gpu.h:506`).
pub const NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2: u32 = 0x0080_0292;
/// `NV0080_CTRL_GPU_CLASSLIST_MAX_SIZE` (`ctrl0080gpu.h:504`).
pub const CLASSLIST_MAX: usize = 200;
/// `sizeof(NV0080_CTRL_GPU_GET_CLASSLIST_V2_PARAMS)` — `{numClasses, classList[200]}`.
pub const CLASSLIST_V2_SIZE: usize = 4 + 4 * CLASSLIST_MAX;

/// Decode a `GET_CLASSLIST_V2` reply. `None` if `numClasses` exceeds the array.
#[must_use]
pub fn decode_classlist(buf: &[u8]) -> Option<Vec<u32>> {
    let w = |o: usize| buf.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let n = w(0)? as usize;
    if n > CLASSLIST_MAX {
        return None;
    }
    (0..n).map(|i| w(4 + 4 * i)).collect()
}

/// A kind the host lists none of — refused by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLacksKind {
    /// The family.
    pub family: Family,
    /// The kind.
    pub kind: Kind,
}

/// ★ The host class profile for THIS die: family set ∩ host list, newest per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedHostClasses {
    family: Family,
    channel: u32,
    usermode: u32,
    ce: u32,
    compute: Option<u32>,
}

impl DerivedHostClasses {
    /// Choose for `family` given the host's own class list.
    ///
    /// # Errors
    /// [`HostLacksKind`] when the host lists no class of a REQUIRED kind (channel, usermode, CE).
    /// Compute is optional (a CE-only host still runs the copy planes).
    pub fn choose(family: Family, host: &[u32]) -> Result<DerivedHostClasses, HostLacksKind> {
        let set = classes_for(family);
        let newest = |k: Kind| set.of_kind(k).iter().copied().filter(|c| host.contains(c)).max();
        let need = |k: Kind| newest(k).ok_or(HostLacksKind { family, kind: k });
        Ok(DerivedHostClasses {
            family,
            channel: need(Kind::ChannelGpfifo)?,
            usermode: need(Kind::Usermode)?,
            ce: need(Kind::DmaCopy)?,
            compute: newest(Kind::Compute),
        })
    }
}

impl HostClasses for DerivedHostClasses {
    fn name(&self) -> &'static str {
        match self.family {
            Family::Turing => "Turing (derived: family set ∩ host class list)",
            Family::Ampere => "Ampere (derived: family set ∩ host class list)",
            Family::Ada => "Ada (derived: family set ∩ host class list)",
            Family::Hopper => "Hopper (derived: family set ∩ host class list)",
            Family::Blackwell => "Blackwell (derived: family set ∩ host class list)",
        }
    }
    fn gpfifo_channel(&self) -> ChannelClass {
        ChannelClass::new(ClassId(self.channel))
    }
    fn usermode(&self) -> UsermodeClass {
        UsermodeClass::new(ClassId(self.usermode))
    }
    fn ce_object(&self) -> CeObjectClass {
        CeObjectClass::new(ClassId(self.ce))
    }
    fn compute_object(&self) -> Option<ComputeObjectClass> {
        self.compute.map(|c| ComputeObjectClass::new(ClassId(c)))
    }
}
