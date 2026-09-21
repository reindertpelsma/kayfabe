//! The RM object graph — §2.2, and the default-deny that makes it a boundary.
//!
//! ## What this is
//!
//! Every RM object the guest creates is a node in one handle namespace per client. The guest
//! allocates a root, a device under it, a subdevice, VA spaces, channel groups, channels and
//! engine objects — and every later control command names one of those handles.
//!
//! ## ★★★ DEFAULT-DENY, and three classes denied BY NAME with a reason
//!
//! §2.2 refuses `NV01_MEMORY_LOCAL_PRIVILEGED` (privileged video memory),
//! `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` — ★ *"would hand the host a **guest-chosen pointer**"* —
//! and `NV40_I2C` (no physical board bus exists).
//!
//! ⊘⊘ **And §2.2 flags its own justification for the third as UNVERIFIED**: the claim that *"RM's
//! own source expects this alloc to fail"* does not check out, because `i2capiConstruct_IMPL`
//! returns `NV_OK` unconditionally. ⚠ The refusal may still be right; its stated reason is not.
//! That is carried here rather than quietly repaired, because a refusal resting on a wrong reason
//! is a refusal nobody can re-derive.
//!
//! ⊘ The `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR` denial is the one that matters for the pointer
//! discipline (`crate::hostverb`): it is the class that would carry a caller pointer, and §6.4
//! calls the denial *"decorative"* because the class never reaches us — the real guard is the
//! leaf bound. ⇒ Both exist; neither is load-bearing alone.

use std::collections::BTreeMap;

pub type ClassId = u32;
pub type HandleId = u32;

/// §2.2's table, as the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassPolicy {
    /// Modelled as a graph node; we answer for it.
    Emulate,
    /// Modelled **and** allocated for real on the host.
    EmulateAndHost,
    /// Admitted past default-deny, tracked as a node, **no facts extracted**.
    OpaqueAllow,
    /// ⊘ Denied by name, with a reason.
    Deny(&'static str),
}

/// The classes §2.2 enumerates. ⊘ Anything not here is denied by default.
pub fn class_policy(class: ClassId) -> ClassPolicy {
    match class {
        0x0 | 0x41 => ClassPolicy::Emulate,     // NV01_ROOT / NV01_ROOT_CLIENT
        0x80 => ClassPolicy::Emulate,           // NV01_DEVICE_0
        0x2080 => ClassPolicy::Emulate,         // NV20_SUBDEVICE_0
        // ⊘ Opaque by design: its controls tunnel whole to firmware, uninterpreted by the
        // kernel. ★ load-bearing for `cuInit`.
        0x2081 => ClassPolicy::OpaqueAllow,     // NV2081_BINAPI
        0x79 => ClassPolicy::Emulate,           // NV01_EVENT_OS_EVENT
        // ⊘ params are a guest-KERNEL function pointer and are deliberately not decoded.
        0x7e => ClassPolicy::Emulate,           // NV01_EVENT_KERNEL_CALLBACK_EX
        0x90f1 => ClassPolicy::EmulateAndHost,  // FERMI_VASPACE_A
        0xa06c => ClassPolicy::Emulate,         // KEPLER_CHANNEL_GROUP_A (TSG)
        0x9067 => ClassPolicy::Emulate,         // FERMI_CONTEXT_SHARE_A
        0xc56f => ClassPolicy::EmulateAndHost,  // AMPERE_CHANNEL_GPFIFO_A
        0xc7c0 => ClassPolicy::EmulateAndHost,  // AMPERE_COMPUTE_B
        0xc797 => ClassPolicy::Emulate,         // AMPERE_B
        0xc7b5 => ClassPolicy::EmulateAndHost,  // AMPERE_DMA_COPY_B
        // ★ The object whose 64 KiB CPU mapping IS the doorbell page.
        0xc561 => ClassPolicy::EmulateAndHost,  // AMPERE_USERMODE_A
        0xc574 => ClassPolicy::Emulate,         // UVM_CHANNEL_RETAINER — ⊘ never forwarded
        // ⊘⊘ [MEASURED] all 4 requests in a boot refused 0x56.
        0xc076 => ClassPolicy::Deny("GP100_UVM_SW: measured refused in every boot"),
        0x3f => ClassPolicy::Deny("NV01_MEMORY_LOCAL_PRIVILEGED: privileged video memory"),
        // ★★★ The pointer one.
        0x71 => ClassPolicy::Deny(
            "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR: would hand the host a guest-chosen pointer",
        ),
        // ⚠ §2.2 marks this justification UNVERIFIED: i2capiConstruct_IMPL returns NV_OK
        // unconditionally, so "RM expects it to fail" is not true. Refusal kept, reason flagged.
        0x402c => ClassPolicy::Deny("NV40_I2C: no physical board bus [reason UNVERIFIED w821]"),
        _ => ClassPolicy::Deny("not on the allowlist — default deny"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub class: ClassId,
    pub parent: Option<HandleId>,
    /// Refcount, because `DUP_OBJECT` **aliases**; it does not copy.
    pub refs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphError {
    DeniedClass(&'static str),
    DuplicateHandle,
    UnknownHandle,
    UnknownParent,
}

/// One client's handle namespace.
#[derive(Debug, Default)]
pub struct ObjectGraph {
    nodes: BTreeMap<HandleId, Node>,
    denied: u64,
}

impl ObjectGraph {
    pub fn new() -> ObjectGraph {
        ObjectGraph::default()
    }

    /// `GSP_RM_ALLOC`. ⊘ **MIXED, default REFUSE** — serving the envelope is not serving its
    /// contents.
    pub fn alloc(&mut self, h: HandleId, class: ClassId, parent: Option<HandleId>) -> Result<ClassPolicy, GraphError> {
        let policy = class_policy(class);
        if let ClassPolicy::Deny(why) = policy {
            self.denied += 1;
            return Err(GraphError::DeniedClass(why));
        }
        if self.nodes.contains_key(&h) {
            return Err(GraphError::DuplicateHandle);
        }
        if let Some(p) = parent {
            if !self.nodes.contains_key(&p) {
                return Err(GraphError::UnknownParent);
            }
        }
        self.nodes.insert(h, Node { class, parent, refs: 1 });
        Ok(policy)
    }

    /// `DUP_OBJECT`. ⊘ §1.3: *"It **aliases, refcount++; it does not copy** — this is the normal
    /// UVM flow."* ⚠ Treating it as a copy is how two namespaces end up with independent
    /// lifetimes for one host object.
    pub fn dup(&mut self, src: HandleId, dst: HandleId) -> Result<(), GraphError> {
        let node = *self.nodes.get(&src).ok_or(GraphError::UnknownHandle)?;
        if self.nodes.contains_key(&dst) {
            return Err(GraphError::DuplicateHandle);
        }
        self.nodes.get_mut(&src).unwrap().refs += 1;
        self.nodes.insert(dst, node);
        Ok(())
    }

    /// `FREE`. ⊘ Frees the subtree, because RM's teardown is ordered by its own dependency rules
    /// and a child outliving its parent is a dangling host object.
    pub fn free(&mut self, h: HandleId) -> Result<usize, GraphError> {
        if !self.nodes.contains_key(&h) {
            return Err(GraphError::UnknownHandle);
        }
        let mut doomed = vec![h];
        let mut i = 0;
        while i < doomed.len() {
            let cur = doomed[i];
            for (k, n) in self.nodes.iter() {
                if n.parent == Some(cur) && !doomed.contains(k) {
                    doomed.push(*k);
                }
            }
            i += 1;
        }
        for d in &doomed {
            self.nodes.remove(d);
        }
        Ok(doomed.len())
    }

    #[inline]
    pub fn get(&self, h: HandleId) -> Option<Node> {
        self.nodes.get(&h).copied()
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// ⚠ Counted, so a zero is evidence rather than silence.
    #[inline]
    pub fn denied(&self) -> u64 {
        self.denied
    }
}
