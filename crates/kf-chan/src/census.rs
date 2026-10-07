// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ P1+P2 inc A (`docs/design/V3_P1P2_TSPACE.md` §3.6) — **the Translated method census.**
//!
//! A dedicated, COUNT-ONLY instrument: what a guest kernel's copy-engine channel actually pushes,
//! per `(bound CE class, subchannel kind, method)`, with the distinct `LAUNCH_DMA` words, the
//! semaphore and `MEM_OP_D` operation values, the header forms and the GP-entry kinds. It exists
//! because the per-tier method tables of the T-mode rewriter cannot be generated from the trimmed
//! open class headers (`clc9b5.h` defines no method at all; `clc86f.h` lists no `SEMAPHOREA-D`), so
//! what stock CeUtils and UVM emit per family must be counted on hardware before an unclassified
//! method is refused there.
//!
//! ⊘ It is not `KF3_COMPLETION_PROBE`, which keeps four records and drops the rest. Nothing here
//! changes a word the rewriter emits, and every map is bounded ([`Census::CAP`]): the input is
//! guest-controlled, so a key past the cap is counted in [`Census::overflow`], never stored.

use std::collections::BTreeMap;

/// Which part of the channel a method write reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubKind {
    /// A host (channel) method — below `0x100`, any subchannel.
    Host,
    /// A copy-engine method on a hardware subchannel (0-4).
    Ce,
    /// A method on a subchannel bound to the `GP100_UVM_SW` software class.
    Sw,
    /// A method on an unbound software subchannel (5-7).
    Unbound,
}

impl SubKind {
    const fn tag(self) -> &'static str {
        match self {
            SubKind::Host => "host",
            SubKind::Ce => "ce",
            SubKind::Sw => "sw",
            SubKind::Unbound => "unbound",
        }
    }
}

/// Which operation field a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpKind {
    /// Host `SEMAPHORED_OPERATION`.
    SemaphoreD,
    /// Host `SEM_EXECUTE_OPERATION`.
    SemExecute,
    /// Host `MEM_OP_D_OPERATION`.
    MemOpD,
}

impl OpKind {
    const fn tag(self) -> &'static str {
        match self {
            OpKind::SemaphoreD => "semd",
            OpKind::SemExecute => "semx",
            OpKind::MemOpD => "memop",
        }
    }
}

/// Which GP entry the ring fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GpKind {
    /// An entry naming method words.
    Segment,
    /// A control entry with this `GP_ENTRY1_OPCODE`.
    Control(u32),
}

/// ★ One channel's census. See the module docs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Census {
    methods: BTreeMap<(u32, SubKind, u32), u64>,
    launches: BTreeMap<(u32, u32), u64>,
    ops: BTreeMap<(OpKind, u32), u64>,
    forms: BTreeMap<&'static str, u64>,
    gp: BTreeMap<GpKind, u64>,
    /// Records refused a new key because their map was full ([`Census::CAP`]).
    pub overflow: u64,
}

fn bump<K: Ord>(m: &mut BTreeMap<K, u64>, k: K, overflow: &mut u64) {
    if let Some(n) = m.get_mut(&k) {
        *n = n.saturating_add(1);
    } else if m.len() < Census::CAP {
        m.insert(k, 1);
    } else {
        *overflow = overflow.saturating_add(1);
    }
}

impl Census {
    /// Distinct keys kept per map (the input is guest-controlled).
    pub const CAP: usize = 256;

    /// A method write `(sub kind, method)` on a channel whose CE class is `class`.
    pub fn method(&mut self, class: u32, kind: SubKind, method: u32) {
        bump(&mut self.methods, (class, kind, method), &mut self.overflow);
    }

    /// A `LAUNCH_DMA` word as the guest wrote it.
    pub fn launch(&mut self, class: u32, word: u32) {
        bump(&mut self.launches, (class, word), &mut self.overflow);
    }

    /// An operation value.
    pub fn op(&mut self, kind: OpKind, value: u32) {
        bump(&mut self.ops, (kind, value), &mut self.overflow);
    }

    /// A header form (`inc`, `non`, `one`, `imm`, `end`, `legacy`, `sdm`).
    pub fn form(&mut self, form: &'static str) {
        bump(&mut self.forms, form, &mut self.overflow);
    }

    /// A GP entry.
    pub fn gp(&mut self, kind: GpKind) {
        bump(&mut self.gp, kind, &mut self.overflow);
    }

    /// Writes counted for `(class, kind, method)`.
    #[must_use]
    pub fn count(&self, class: u32, kind: SubKind, method: u32) -> u64 {
        self.methods
            .get(&(class, kind, method))
            .copied()
            .unwrap_or(0)
    }

    /// Times `word` was launched on `class`.
    #[must_use]
    pub fn launches_of(&self, class: u32, word: u32) -> u64 {
        self.launches.get(&(class, word)).copied().unwrap_or(0)
    }

    /// Times `value` was seen in `kind`.
    #[must_use]
    pub fn ops_of(&self, kind: OpKind, value: u32) -> u64 {
        self.ops.get(&(kind, value)).copied().unwrap_or(0)
    }

    /// GP entries of `kind` fetched.
    #[must_use]
    pub fn gp_of(&self, kind: GpKind) -> u64 {
        self.gp.get(&kind).copied().unwrap_or(0)
    }

    /// Headers of `form` decoded.
    #[must_use]
    pub fn forms_of(&self, form: &str) -> u64 {
        self.forms.get(form).copied().unwrap_or(0)
    }

    /// Nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.methods.is_empty() && self.gp.is_empty() && self.overflow == 0
    }

    /// ★ The one line dumped per channel at free:
    /// `methods=[ce/0xc7b5:0x300x12 …] launch=[0xc7b5:0x182x4 …] ops=[semd:0x2x3 …]
    /// forms=[inc:10 …] gp=[seg:12 ctl0:1 …] overflow=0`.
    #[must_use]
    pub fn line(&self) -> String {
        let methods: Vec<String> = self
            .methods
            .iter()
            .map(|(&(c, k, m), n)| format!("{}/{c:#x}:{m:#x}x{n}", k.tag()))
            .collect();
        let launches: Vec<String> = self
            .launches
            .iter()
            .map(|(&(c, w), n)| format!("{c:#x}:{w:#x}x{n}"))
            .collect();
        let ops: Vec<String> = self
            .ops
            .iter()
            .map(|(&(k, v), n)| format!("{}:{v:#x}x{n}", k.tag()))
            .collect();
        let forms: Vec<String> = self.forms.iter().map(|(f, n)| format!("{f}:{n}")).collect();
        let gp: Vec<String> = self
            .gp
            .iter()
            .map(|(k, n)| match k {
                GpKind::Segment => format!("seg:{n}"),
                GpKind::Control(o) => format!("ctl{o}:{n}"),
            })
            .collect();
        format!(
            "methods=[{}] launch=[{}] ops=[{}] forms=[{}] gp=[{}] overflow={}",
            methods.join(" "),
            launches.join(" "),
            ops.join(" "),
            forms.join(" "),
            gp.join(" "),
            self.overflow
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cap holds: a guest that streams distinct methods cannot grow the record past
    /// [`Census::CAP`] keys per map; the rest is counted, never stored.
    #[test]
    fn a_guest_cannot_grow_the_census_past_its_cap() {
        let mut c = Census::default();
        for m in 0..(Census::CAP as u32 + 10) {
            c.method(0xc7b5, SubKind::Ce, 0x100 + 4 * m);
        }
        assert_eq!(c.methods.len(), Census::CAP);
        assert_eq!(c.overflow, 10);
        // A key already present still counts past the cap.
        c.method(0xc7b5, SubKind::Ce, 0x100);
        assert_eq!(c.count(0xc7b5, SubKind::Ce, 0x100), 2);
        assert_eq!(c.overflow, 10);
    }

    #[test]
    fn the_line_names_every_map() {
        let mut c = Census::default();
        c.method(0xc7b5, SubKind::Host, 0x10);
        c.launch(0xc7b5, 0x182);
        c.op(OpKind::SemaphoreD, 2);
        c.form("inc");
        c.gp(GpKind::Segment);
        c.gp(GpKind::Control(4));
        let l = c.line();
        for want in [
            "host/0xc7b5:0x10x1",
            "0xc7b5:0x182x1",
            "semd:0x2x1",
            "inc:1",
            "seg:1",
            "ctl4:1",
            "overflow=0",
        ] {
            assert!(l.contains(want), "{want} missing from {l}");
        }
    }
}
