//! Exact-cell wire evidence for the default-off software-runlist allocation diagnostic.
//! The public SDK defines the parameters, but not this retail class number. The
//! generated registration evidence is deliberately separate from normal capabilities.

#[path = "sw_runlist_generated.rs"]
mod generated;

#[path = "sw_runlist_native_generated.rs"]
mod native;

/// Public allocation layout and exact Windows identity, without scheduling evidence.
#[derive(Debug, Clone, Copy)]
pub struct AllocationCell {
    /// Public wire-layout tag associated with the Windows build.
    pub wire_version: crate::DriverVersion,
    /// Exact published Windows build name.
    pub windows_name: &'static str,
    /// CODE READ: historical text at commit 007ba1eb0:18;
    /// BW9 quotes this statement without independently validating its evidence.
    /// Measured external allocation class, not a public SDK define.
    pub class: u32,
    /// Compiler-derived parameter bytes.
    pub size: usize,
    /// Compiler-derived `engineId` offset.
    pub engine: usize,
    /// Compiler-derived `maxTSGs` offset.
    pub max_tsgs: usize,
    /// Compiler-derived `qosIntrEnableMask` offset.
    pub qos: usize,
}

/// CODE READ: historical text at commit 007ba1eb0:30;
/// BW9 quotes this statement without independently validating its evidence.
/// A measurement is scoped to its exact host die and guest build.
pub(super) struct MeasuredAllocationCell {
    pub architecture: u32,
    pub implementation: u32,
    pub allocation: AllocationCell,
}

/// One compiled layout and exact retail registration; no family/die facts.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceCell {
    /// Public wire-layout tag, independently associated with the Windows build.
    pub wire_version: crate::DriverVersion,
    /// Published Windows build name.
    pub windows_name: &'static str,
    /// Retail registration's external class number (not an OGKM define).
    pub class: u32,
    /// Compiler-derived parameter bytes.
    pub size: usize,
    /// Compiler-derived `engineId` offset.
    pub engine: usize,
    /// Compiler-derived `maxTSGs` offset.
    pub max_tsgs: usize,
    /// Compiler-derived `qosIntrEnableMask` offset.
    pub qos: usize,
    /// Pinned retail caller's control code; observational only, never an allowlist.
    pub observed_control: u32,
    /// Pinned retail caller's exact parameter size for that control.
    pub observed_control_size: usize,
    /// Compiled fixed `rpc_alloc_memory_v13_01` size; its PTE tail is not decoded.
    pub alloc_memory_size: usize,
    /// Compiled scalar prefix fields only: name, offset, width. No PTE/address fields.
    pub alloc_memory_fields: &'static [(&'static str, usize, usize)],
}

/// The complete public three-word allocation declaration; all values are untrusted.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// `NV2080_ENGINE_TYPE_*`, not an RM engine or physical runlist index.
    pub engine: u32,
    /// Native zero selects hardware default capacity.
    pub max_tsgs: u32,
    /// Requested QoS interrupt mask.
    pub qos: u32,
}

/// Look up only evidenced driver cells. No nearest-version or per-die fallback.
#[must_use]
pub fn cell(version: crate::DriverVersion) -> Option<&'static EvidenceCell> {
    generated::CELLS.iter().find(|c| c.wire_version == version)
}

/// Allocation-only diagnostic vocabulary for an exact host die and guest build.
///
/// This never adds a scheduling registration to [`cell`]. Existing legacy cells
/// retain their scope; native measurements do not extend to adjacent dies/builds.
#[must_use]
pub fn allocation_cell(
    version: crate::DriverVersion,
    architecture: u32,
    implementation: u32,
) -> Option<AllocationCell> {
    cell(version).map(EvidenceCell::allocation).or_else(|| {
        native::CELLS.iter().find_map(|c| {
            (c.allocation.wire_version == version
                && c.architecture == architecture
                && c.implementation == implementation)
                .then_some(c.allocation)
        })
    })
}

/// Recognize the diagnostic's class even on unsupported wire cells, to refuse it.
#[must_use]
pub fn is_probe_class(class: u32) -> bool {
    generated::CELLS.iter().any(|c| c.class == class)
        || native::CELLS.iter().any(|c| c.allocation.class == class)
}

impl EvidenceCell {
    /// Allocation vocabulary, independently of this legacy cell's observations.
    #[must_use]
    pub const fn allocation(&self) -> AllocationCell {
        AllocationCell {
            wire_version: self.wire_version,
            windows_name: self.windows_name,
            class: self.class,
            size: self.size,
            engine: self.engine,
            max_tsgs: self.max_tsgs,
            qos: self.qos,
        }
    }

    /// This identity selects an evidence cell, never a privilege/security boundary.
    #[must_use]
    pub fn matches_identity(&self, payload: &[u8]) -> bool {
        self.allocation().matches_identity(payload)
    }

    /// Decode the legacy allocation layout without authorizing scheduling.
    #[must_use]
    pub fn decode(&self, params: &[u8]) -> Option<Params> {
        self.allocation().decode(params)
    }
}

impl AllocationCell {
    /// An exact identity selects vocabulary, never host authority.
    #[must_use]
    pub fn matches_identity(&self, payload: &[u8]) -> bool {
        crate::guestsysinfo::ReportedDriver::decode(payload).is_ok_and(|id| {
            id.version == Some(self.wire_version)
                && id.twin.is_some_and(|t| t.win_name == self.windows_name)
        })
    }

    /// Decode exactly the compiled layout. Feature policy belongs to the caller.
    #[must_use]
    pub fn decode(&self, params: &[u8]) -> Option<Params> {
        if params.len() != self.size {
            return None;
        }
        let read = |off: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                params.get(off..off.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        Some(Params {
            engine: read(self.engine)?,
            max_tsgs: read(self.max_tsgs)?,
            qos: read(self.qos)?,
        })
    }
}
