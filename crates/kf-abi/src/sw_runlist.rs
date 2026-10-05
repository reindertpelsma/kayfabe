//! Exact-cell wire evidence for the default-off software-runlist allocation diagnostic.
//! The public SDK defines the parameters, but not this retail class number. The
//! generated registration evidence is deliberately separate from normal capabilities.

#[path = "sw_runlist_generated.rs"]
mod generated;

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

/// Recognize the diagnostic's class even on unsupported wire cells, to refuse it.
#[must_use]
pub fn is_probe_class(class: u32) -> bool {
    generated::CELLS.iter().any(|c| c.class == class)
}

impl EvidenceCell {
    /// This identity selects an evidence cell, never a privilege/security boundary.
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
