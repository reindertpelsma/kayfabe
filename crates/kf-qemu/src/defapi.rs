//! ★★ OWNER_RULINGS §U (2026-10-07) — **the deferred API on the channel plane: Translated only,
//! served by host-authored equivalents.** The decisions, apart from the plane's maps and host
//! session, so each is a function a test drives (`crate::chan` keeps the maps and calls these).
//!
//! - **Admission** ([`admit`]): an `NV50_DEFERRED_API_CLASS` alloc under a Translated channel is
//!   admitted and numbered; under a Passthrough twin it is refused `NV_ERR_NOT_SUPPORTED` (§U.1:
//!   a Passthrough ring is never read, so its `0x200` could only be forwarded as guest bytes).
//! - **Numbering** ([`SwObjs`]): the guest's CPU-RM gives every `ENG_SW` child of a channel the
//!   channel's next software classID BEFORE it RPCs the alloc (`kchannelRegisterChild`,
//!   `ogkm-580: kernel_channel.c:3408-3453`), and its `SET_OBJECT` names the object by that number
//!   (`kchannelGetClassEngineID_GM107`, `kernel_channel_gm107.c:72-82`). `[measured: the 2026-10-05 VFIO boots 8/9/10]`
//!   the 5080 is the first and only `ENG_SW` child of every Windows channel, and runs 31-46 bind
//!   subchannel 5 to `1`. The plane counts every `ENG_SW` child of a Translated channel, refused
//!   or not, exactly as `crate::dispsw::SwClassIds` does for twins.
//! - **The trigger** ([`classify`]): a method on a subchannel bound to one of the channel's own
//!   objects: `NO_OPERATION` (`0x100-0x103`) consumed; `0x200-0x203` (`_class5080DeferredApiV2`)
//!   looks `data` up in THAT object's table (`kf_rm::defapi`) and plans the host-authored action;
//!   anything else is refused by name and kills the channel (on hardware: an RC, Xid 32).
//! - **The actions** ([`ctx_verdict`]): which of the eight commands kayfabe performs, and how.
//!
//! | command | on the trigger |
//! |---|---|
//! | `DMA_INVALIDATE_TLB` | the host GATE (`kf_chan::swmethod`): walk + commit + host invalidate of the channel's OWN space by the VA-manager thread, which then opens the gate; the guest's `hClientVA`/`hDeviceVA`/`hVASpace` are ignored (§U.2) |
//! | `GPU_INITIALIZE_CTX` | satisfied by the twin (owner ruling B, "golden context is guest-kernel-only"): the target's host context was created and golden-initialised by host RM at its birth; `PRESERVE_CTX` (a context migrated from another channel) is REFUSED |
//! | `GPU_PROMOTE_CTX` | satisfied by the twin (ruling B), the legacy `(virtAddress, size)` shape only; `hVirtMemory` and entry lists are REFUSED (no producer measured) |
//! | `GPU_EVICT_CTX` | the target's host ring off its runlist — the direct path's own act (`ChanPlane::evict_ctx`) |
//! | `FIFO_UPDATE_CHANNEL_INFO` | REFUSED: re-pointing a channel's USERD/GPFIFO has no unprivileged host verb |
//! | `GR_CTXSW_ZCULL_BIND` | REFUSED: the direct path serves Passthrough twins only (VA identity); a Translated twin runs in the T-space, which maps no guest VA |
//! | `GR_CTXSW_PM_BIND` | REFUSED: a PM context buffer at a guest VA — same reason; and no PM feature |
//! | `GR_CTXSW_PREEMPTION_BIND` | REFUSED: preemption buffers at guest VAs — same reason |
//!
//! ⊘ A refusal never leaves the entry behind silently: the real trigger consumes it whatever the
//! status (`deferred_api.c:653-670`), so does this one, and the channel dies by name.

use kf_abi::defapi::Bundle;
use kf_chan::swmethod::SwKind;
use kf_chan::tmode::SwCall;
use kf_rm::defapi::{Entry, ObjKey, Registry};
use std::collections::HashMap;

/// `NV50_DEFERRED_API_CLASS`.
pub const NV50_DEFERRED_API_CLASS: u32 = kf_abi::generated::classes::NV50_DEFERRED_API_CLASS;
/// `NV_ERR_NOT_SUPPORTED`.
pub const NV_ERR_NOT_SUPPORTED: u32 = 0x56;
/// `NV_ERR_INVALID_ARGUMENT`.
pub const NV_ERR_INVALID_ARGUMENT: u32 = 0x1f;

/// `mthdNoOperation` `0x100-0x103` and `_class5080DeferredApiV2` `0x200-0x203`
/// (`deferred_api.c:676-680`).
const NOP_METHODS: core::ops::RangeInclusive<u32> = 0x100..=0x103;
const DEFERRED_METHODS: core::ops::RangeInclusive<u32> = 0x200..=0x203;

/// ★ One Translated channel's software objects, by the classID the guest gave each.
#[derive(Debug, Default, Clone)]
pub struct SwObjs {
    /// The channel `(hClient, hChannel)`.
    pub owner: (u32, u32),
    ids: crate::dispsw::SwClassIds,
    /// classID → `(class, handle)`; a number the mirror lost (past 65535) is never recorded.
    by_id: HashMap<u32, (u32, u32)>,
    /// `ENG_SW` children numbered.
    pub numbered: u64,
}

impl SwObjs {
    /// The objects of channel `owner`, none yet.
    #[must_use]
    pub fn new(owner: (u32, u32)) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }

    /// The guest numbered one more `ENG_SW` child (`handle`, of `class`): its number, if the
    /// mirror still knows it.
    pub fn number(&mut self, class: u32, handle: Option<u32>) -> Option<u16> {
        self.numbered += 1;
        let n = self.ids.register()?;
        if let Some(h) = handle {
            self.by_id.insert(u32::from(n), (class, h));
        }
        Some(n)
    }

    /// The object `SET_OBJECT` value `v` names.
    #[must_use]
    pub fn object(&self, v: u32) -> Option<(u32, u32)> {
        self.by_id.get(&v).copied()
    }

    /// `handle` was freed: its number is never reused by the guest (the counter only climbs).
    pub fn forget(&mut self, handle: u32) -> bool {
        let before = self.by_id.len();
        self.by_id.retain(|_, (_, h)| *h != handle);
        before != self.by_id.len()
    }
}

/// What the plane does with a 5080 alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Admitted: its software classID on the channel.
    Admitted(Option<u16>),
    /// Refused, by name.
    Refused(u32, String),
}

/// ★ §U.1 — a 5080 alloc: `translated` — the parent is one of the plane's Translated channels
/// (its objects); `passthrough` — the parent is a Passthrough twin.
pub fn admit(
    translated: Option<&mut SwObjs>,
    passthrough: bool,
    client: u32,
    parent: u32,
    handle: u32,
) -> Admission {
    match translated {
        Some(o) => Admission::Admitted(o.number(NV50_DEFERRED_API_CLASS, Some(handle))),
        None if passthrough => Admission::Refused(
            NV_ERR_NOT_SUPPORTED,
            format!(
                "NV50_DEFERRED_API_CLASS {client:#x}:{handle:#x} under PASSTHROUGH channel {parent:#x}: \
                 Translated-only (OWNER_RULINGS §U.1) — a Passthrough ring is never read, so its 0x200 \
                 could only reach the host as guest bytes"
            ),
        ),
        None => Admission::Refused(
            NV_ERR_NOT_SUPPORTED,
            format!(
                "NV50_DEFERRED_API_CLASS {client:#x}:{handle:#x} under {parent:#x}: no Translated channel of ours (§U.1)"
            ),
        ),
    }
}

/// A planned action: the object and entry it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The 5080 object.
    pub key: ObjKey,
    /// The entry.
    pub entry: Entry,
    /// The bundle, decoded.
    pub bundle: Bundle,
}

/// ★ The trigger's classification of `call` on a channel whose objects are `objs`, against the
/// VM's tables `reg` decoded with `abi`. A refusal consumes the entry it named (the real cleanup
/// runs whatever the status) and kills the channel by name.
///
/// # Errors
/// The refusal, by name.
pub fn classify(
    objs: Option<&SwObjs>,
    reg: &Registry,
    call: &SwCall,
) -> Result<(SwKind, Option<Planned>), String> {
    let objs = objs.ok_or_else(|| {
        format!(
            "software method {:#x} on subchannel {}: the channel has no software-object record",
            call.method, call.sub
        )
    })?;
    let (class, handle) = objs.object(call.value).ok_or_else(|| {
        format!(
            "software subchannel {} bound to {:#x}: no software object of this channel has that number \
             — method {:#x} refused (on hardware: an RC, Xid 32)",
            call.sub, call.value, call.method
        )
    })?;
    if class != NV50_DEFERRED_API_CLASS {
        return Err(format!(
            "software object {handle:#x} (class {class:#x}) on subchannel {}: its method {:#x} is not served \
             (§S GR ruling 4)",
            call.sub, call.method
        ));
    }
    if NOP_METHODS.contains(&call.method) {
        return Ok((SwKind::Nop, None));
    }
    if !DEFERRED_METHODS.contains(&call.method) {
        return Err(format!(
            "NV50_DEFERRED_API {handle:#x}: method {:#x} is not in its method table (deferred_api.c:676-680)",
            call.method
        ));
    }
    let key = ObjKey {
        client: objs.owner.0,
        object: handle,
    };
    let entry = reg.lookup(key, call.data).map_err(|st| {
        format!(
            "NV50_DEFERRED_API {:#x}:{handle:#x}: hApiHandle {:#x} is not registered on this object \
             (NV_ERR_INVALID_DATA {st:#x}) — the channel dies, as on hardware (Xid 32)",
            key.client, call.data
        )
    })?;
    let bundle = entry.decoded;
    let refuse = |why: String| -> Result<(SwKind, Option<Planned>), String> {
        // The real cleanup: executed + implicit delete, whatever the status.
        reg.executed(key, call.data, false);
        Err(format!(
            "NV50_DEFERRED_API {:#x}:{handle:#x} hApiHandle {:#x} cmd {:#x}: {why}",
            key.client, call.data, entry.cmd
        ))
    };
    let kind = match bundle {
        Bundle::InvalidateTlb { .. } => SwKind::Act { gate: true },
        Bundle::InitializeCtx { .. } | Bundle::PromoteCtx { .. } | Bundle::EvictCtx { .. } => {
            SwKind::Act { gate: false }
        }
        Bundle::ZcullBind { .. } => {
            return refuse(
                "GR_CTXSW_ZCULL_BIND REFUSED: the direct path serves Passthrough twins only (VA identity); a \
                 Translated twin runs in the T-space, which maps no guest VA"
                    .into(),
            );
        }
        Bundle::Unserved { what, .. } => {
            let why = match what {
                "GR_CTXSW_PM_BIND" => {
                    "a PM context buffer at a guest VA the T-space twin does not map"
                }
                "GR_CTXSW_PREEMPTION_BIND" => {
                    "preemption buffers at guest VAs the T-space twin does not map"
                }
                _ => "re-pointing a channel's USERD/GPFIFO has no unprivileged host verb",
            };
            return refuse(format!("{what} REFUSED: {why}"));
        }
        Bundle::Unknown { .. } => {
            return refuse(
                "not one of the eight deferred commands (NV_ERR_INVALID_ARGUMENT, deferred_api.c:544-551)".into(),
            );
        }
    };
    Ok((kind, Some(Planned { key, entry, bundle })))
}

/// What one target channel is, for [`ctx_verdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Its host token.
    pub ht: u32,
    /// The guest's declared engine.
    pub engine: u32,
    /// Its host ring owns a real context on `engine`.
    pub owns_context: bool,
    /// It is dead.
    pub dead: bool,
}

/// What an INITIALIZE/PROMOTE/EVICT does to its targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtxEffect {
    /// Satisfied by each target's own host context (ruling B): recorded, nothing sent.
    Satisfied,
    /// Each target's host ring goes off its runlist (the direct evict's act).
    Evict,
}

/// ★ Which targets a ctx bundle acts on and how — the direct path's own predicate (`GPU_PROMOTE_CTX`
/// on a Translated channel: the guest engine matches, the ring owns that context, it is alive).
/// `targets`: the plane's Translated channels the bundle's `(hChanClient, hObject)` names (the
/// channel itself, or every member of the TSG it names).
///
/// # Errors
/// The refusal, by name (`NV_ERR_INVALID_ARGUMENT` on hardware for a bad target).
pub fn ctx_verdict(bundle: &Bundle, targets: &[Target]) -> Result<(CtxEffect, Vec<u32>), String> {
    let (engine, effect, what) = match *bundle {
        Bundle::InitializeCtx {
            engine_type,
            phys_attr,
            ..
        } => {
            if (phys_attr >> kf_abi::defapi::INITIALIZE_CTX_PRESERVE_CTX_BIT) & 1 == 1 {
                return Err(
                    "GPU_INITIALIZE_CTX with PRESERVE_CTX: adopting a context initialised on another channel is a \
                     context migration a host twin cannot perform — REFUSED (report for the owner)"
                        .into(),
                );
            }
            (engine_type, CtxEffect::Satisfied, "GPU_INITIALIZE_CTX")
        }
        Bundle::PromoteCtx {
            engine_type,
            virt_memory,
            virt_address,
            size,
            entry_count,
            ..
        } => {
            if entry_count != 0 {
                return Err(format!(
                    "deferred GPU_PROMOTE_CTX with {entry_count} promote entries: no producer measured (Windows 580.88 \
                     sends the (virtAddress, size) shape) — REFUSED"
                ));
            }
            if virt_memory != 0 || virt_address == 0 || size == 0 {
                return Err(format!(
                    "deferred GPU_PROMOTE_CTX hVirtMemory={virt_memory:#x} va={virt_address:#x} size={size:#x}: only the \
                     (virtAddress, size) shape is served — REFUSED"
                ));
            }
            (engine_type, CtxEffect::Satisfied, "GPU_PROMOTE_CTX")
        }
        Bundle::EvictCtx { engine_type, .. } => (engine_type, CtxEffect::Evict, "GPU_EVICT_CTX"),
        _ => return Err("not a context command".into()),
    };
    let hit: Vec<u32> = targets
        .iter()
        .filter(|t| t.engine == engine)
        .map(|t| t.ht)
        .collect();
    if hit.is_empty() {
        return Err(format!(
            "deferred {what} engine {engine:#x}: the named channel/TSG has no Translated channel of ours on that engine \
             ({} candidate(s))",
            targets.len()
        ));
    }
    if let Some(t) = targets
        .iter()
        .find(|t| t.engine == engine && (!t.owns_context || t.dead))
    {
        return Err(format!(
            "deferred {what} engine {engine:#x}: host {:#x} owns no live context on it (owns={} dead={})",
            t.ht, t.owns_context, t.dead
        ));
    }
    Ok((effect, hit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_abi::DriverVersion;
    use kf_abi::defapi::{Control, DefApiAbi, Form};

    const W: DriverVersion = DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    };
    const CH: (u32, u32) = (0xc1d0_0015, 0xff04_0001);
    const OBJ: u32 = 0xff1f_e010;

    fn abi() -> DefApiAbi {
        DefApiAbi::at(W).unwrap()
    }

    fn register(reg: &Registry, handle: u32, cmd: u32, bundle: &[u32]) {
        let mut p = vec![0u8; 584];
        p[0..4].copy_from_slice(&handle.to_le_bytes());
        p[4..8].copy_from_slice(&cmd.to_le_bytes());
        for (i, w) in bundle.iter().enumerate() {
            p[24 + 4 * i..28 + 4 * i].copy_from_slice(&w.to_le_bytes());
        }
        let key = ObjKey {
            client: CH.0,
            object: OBJ,
        };
        assert_eq!(
            kf_rm::defapi::serve(reg, &abi(), key, Control::Register(Form::V1), &p, |_| false),
            0
        );
    }

    fn call(value: u32, method: u32, data: u32) -> SwCall {
        SwCall {
            sub: 5,
            value,
            method,
            data,
        }
    }

    #[test]
    fn admission_is_translated_only_and_numbers_the_first_child_one() {
        let mut o = SwObjs::new(CH);
        assert_eq!(
            admit(Some(&mut o), false, CH.0, CH.1, OBJ),
            Admission::Admitted(Some(1))
        );
        assert_eq!(o.object(1), Some((NV50_DEFERRED_API_CLASS, OBJ)));
        let Admission::Refused(st, why) = admit(None, true, CH.0, 0xff04_0099, 0xff1f_e011) else {
            panic!("passthrough admitted");
        };
        assert_eq!(st, NV_ERR_NOT_SUPPORTED);
        assert!(why.contains("PASSTHROUGH") && why.contains("§U.1"));
        assert!(matches!(
            admit(None, false, 1, 2, 3),
            Admission::Refused(0x56, _)
        ));
        // A refused or other ENG_SW child still consumes a number.
        assert_eq!(o.number(0x9074, None), Some(2));
        assert_eq!(
            admit(Some(&mut o), false, CH.0, CH.1, 0xff1f_e012),
            Admission::Admitted(Some(3))
        );
        assert_eq!(o.object(2), None);
        assert!(o.forget(OBJ));
        assert_eq!(o.object(1), None, "freed: its number names nothing");
    }

    /// ★ The run45 trigger against the VFIO registrations: `0x200` = `0x40000002` on subchannel 5
    /// bound to 1 → the deferred INITIALIZE of client c1d0002b's TSG, an un-gated act; `0x100`
    /// is a NOP; an unregistered handle, another method, another number are refused by name.
    #[test]
    fn the_windows_trigger_classifies_to_the_planned_actions() {
        let reg = Registry::new(kf_rm::defapi::Bounds::default());
        register(
            &reg,
            0x4000_0002,
            0x2080_012d,
            &[
                1,
                0xc1d0_0029,
                0x10,
                0xc1d0_002b,
                0xff0e_0000,
                0,
                0x0360_a000,
                0,
                0,
                0,
                0x10,
                0,
            ],
        );
        register(
            &reg,
            0x4000_0003,
            0x2080_012b,
            &[
                1,
                0xc1d0_0029,
                0x10,
                0xc1d0_002b,
                0xff0e_0000,
                0,
                0x11000,
                0,
                0xdc300,
                0,
                0,
            ],
        );
        register(&reg, 0x4000_0004, 0x2080_2502, &[0, 0, 0, 0xbad]);
        register(&reg, 0x4000_0005, 0x2080_1208, &[0, 0, 0, 0, 2]);
        register(&reg, 0x4000_0006, 0x2080_0101, &[]);
        let mut o = SwObjs::new(CH);
        o.number(NV50_DEFERRED_API_CLASS, Some(OBJ));
        let (k, p) = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0002)).unwrap();
        assert_eq!(k, SwKind::Act { gate: false });
        let p = p.unwrap();
        assert!(matches!(
            p.bundle,
            Bundle::InitializeCtx {
                chan_client: 0xc1d0_002b,
                object: 0xff0e_0000,
                ..
            }
        ));
        let (k, _) = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0003)).unwrap();
        assert_eq!(k, SwKind::Act { gate: false });
        let (k, _) = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0004)).unwrap();
        assert_eq!(k, SwKind::Act { gate: true }, "the TLB invalidate is gated");
        assert_eq!(
            classify(Some(&o), &reg, &call(1, 0x100, 0)).unwrap().0,
            SwKind::Nop
        );
        let e = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0099)).unwrap_err();
        assert!(
            e.contains("NV_ERR_INVALID_DATA") && e.contains("Xid 32"),
            "{e}"
        );
        assert!(
            classify(Some(&o), &reg, &call(1, 0x204, 0))
                .unwrap_err()
                .contains("method table")
        );
        assert!(
            classify(Some(&o), &reg, &call(2, 0x200, 0x4000_0002))
                .unwrap_err()
                .contains("no software object")
        );
        assert!(classify(None, &reg, &call(1, 0x200, 0x4000_0002)).is_err());
        // Refused commands consume their entry (the real cleanup), by name.
        let key = ObjKey {
            client: CH.0,
            object: OBJ,
        };
        let e = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0005)).unwrap_err();
        assert!(e.contains("ZCULL_BIND REFUSED"), "{e}");
        assert!(reg.lookup(key, 0x4000_0005).is_err(), "consumed");
        let e = classify(Some(&o), &reg, &call(1, 0x200, 0x4000_0006)).unwrap_err();
        assert!(e.contains("NV_ERR_INVALID_ARGUMENT"), "{e}");
        // A non-5080 software object's methods stay refused (ruling 4).
        let mut o2 = SwObjs::new(CH);
        o2.number(0x9074, Some(0x77));
        assert!(
            classify(Some(&o2), &reg, &call(1, 0x200, 1))
                .unwrap_err()
                .contains("ruling 4")
        );
    }

    #[test]
    fn ctx_verdicts_follow_the_direct_paths_predicate() {
        let gr = |ht, owns, dead| Target {
            ht,
            engine: 1,
            owns_context: owns,
            dead,
        };
        let init = |attr: u32| Bundle::InitializeCtx {
            engine_type: 1,
            h_client: 0,
            chid: 0x10,
            chan_client: 0xc1d0_002b,
            object: 0xff0e_0000,
            virt_memory: 0,
            phys_address: 0x0360_a000,
            phys_attr: attr,
            dma_handle: 0,
            index: 0x10,
            size: 0,
        };
        let ce = Target {
            ht: 9,
            engine: 0x0d,
            owns_context: true,
            dead: false,
        };
        assert_eq!(
            ctx_verdict(&init(0), &[gr(3, true, false), ce]),
            Ok((CtxEffect::Satisfied, vec![3]))
        );
        assert!(
            ctx_verdict(&init(1 << 3), &[gr(3, true, false)])
                .unwrap_err()
                .contains("PRESERVE_CTX")
        );
        assert!(
            ctx_verdict(&init(0), &[ce])
                .unwrap_err()
                .contains("no Translated channel")
        );
        assert!(
            ctx_verdict(&init(0), &[gr(3, false, false)])
                .unwrap_err()
                .contains("owns no live context")
        );
        assert!(ctx_verdict(&init(0), &[gr(3, true, true)]).is_err());
        let promote = |vm: u32, va: u64, n: u32| Bundle::PromoteCtx {
            engine_type: 1,
            h_client: 0,
            chid: 0x10,
            chan_client: 1,
            object: 2,
            virt_memory: vm,
            virt_address: va,
            size: 0xdc300,
            entry_count: n,
        };
        assert_eq!(
            ctx_verdict(&promote(0, 0x11000, 0), &[gr(3, true, false)]),
            Ok((CtxEffect::Satisfied, vec![3]))
        );
        assert!(ctx_verdict(&promote(5, 0x11000, 0), &[gr(3, true, false)]).is_err());
        assert!(ctx_verdict(&promote(0, 0, 0), &[gr(3, true, false)]).is_err());
        assert!(
            ctx_verdict(&promote(0, 0x11000, 2), &[gr(3, true, false)])
                .unwrap_err()
                .contains("entries")
        );
        let evict = Bundle::EvictCtx {
            engine_type: 1,
            h_client: 0,
            chid: 0,
            chan_client: 1,
            object: 2,
        };
        assert_eq!(
            ctx_verdict(&evict, &[gr(3, true, false), gr(4, true, false)]),
            Ok((CtxEffect::Evict, vec![3, 4]))
        );
    }
}
