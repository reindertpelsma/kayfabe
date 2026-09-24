//! ★★★ **The channel plane's seat in the served chain** (`V3_P5_PORT_MAP.md` §2.1).
//!
//! The guest's channels are ALLOCATED, SCHEDULED and TOKENED through RPCs we answer. On the GSP
//! model the guest's CPU-RM does only bookkeeping and RPCs the rest — *"All real hardware
//! management is done in the host"* (`ogkm-580: kernel_channel.c:3105-3130`) — so the runlist
//! write, the RAMFC and the doorbell's meaning are OURS. This link carries each of those
//! statements to the device's channel plane (`kf-qemu`), which births the host twin, and answers
//! with what the plane did:
//!
//! | RPC | this link | the plane |
//! |---|---|---|
//! | `GSP_RM_ALLOC` of the GPFIFO channel class | carries the declaration ([`ChannelAlloc`]); **refuses the alloc** if the plane refused the birth, else declines (the object seat records it) | births the host twin **at allocation** (§7: never lazily) |
//! | `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`0xa06f0103`) | answers `NV_OK` + the `[IN]` echo **iff** the plane scheduled | `GPFIFO_SCHEDULE` on the host twin |
//! | `NVA06F_CTRL_CMD_BIND` (`0xa06f0104`) | answers `NV_OK` + the `[IN]` echo iff the plane owns the twin and the engine is a copy engine | nothing more: the twin's host TSG was bound at birth |
//! | `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`0xc36f0108`) | answers the plane's GUEST token | a token-table slot routed to the twin |
//! | `GSP_RM_FREE` | observes | retires the token, frees the twin |
//!
//! ⊘ **Nothing here is answered without the plane having acted.** `0xa06f0103` stayed unserviced
//! for weeks precisely because an `NV_OK` with no host act behind it is a fabricated completion
//! (`sweep.rs` row `0xa06f_0103`); the answer now IS the act's result.
//!
//! ⊘ A channel this plane does not own (no statement accepted for it) gets `None` from every arm,
//! so the chain's other links and the unserviced ledger see it exactly as before.

use kf_abi::versions::{AllocParams, DriverAbiTable};
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `NV_OK`.
const NV_OK: u32 = 0;
/// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` (`ogkm-580: ctrla06fgpfifo.h:69`).
pub const GPFIFO_SCHEDULE: u32 = 0xa06f_0103;
/// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` — the TSG form (`ctrla06c.h`).
pub const TSG_GPFIFO_SCHEDULE: u32 = 0xa06c_0101;
/// `NVA06F_CTRL_CMD_BIND` (`ogkm-580: ctrla06fgpfifo.h:96`) — one `[IN]` `engineType`.
pub const BIND: u32 = 0xa06f_0104;
/// `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` (`ogkm-580: ctrlc36f.h:79`).
pub const GET_WORK_SUBMIT_TOKEN: u32 = 0xc36f_0108;

/// ★ What the guest declared when it allocated a GPFIFO channel — every field off the wire,
/// nothing inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelAlloc {
    /// `hClient`.
    pub client: u32,
    /// `hParent` (a device, or a TSG).
    pub parent: u32,
    /// The channel's handle.
    pub handle: u32,
    /// The class id.
    pub class: u32,
    /// `gpFifoOffset` — a GPU VA in the channel's VA space.
    pub gpfifo_va: u64,
    /// `gpFifoEntries`.
    pub entries: u32,
    /// `hVASpace` as declared (0 = the device's default / the TSG's).
    pub h_vaspace: u32,
    /// ★ The VA-space object the channel runs in, RESOLVED: `hVASpace` when non-zero, else the
    /// first `FERMI_VASPACE_A` allocated under the channel's parent device in this client (the
    /// device-default VAS RM creates lazily — the PMA scrubber's, `hVASpace = 0`). `None` when
    /// neither is known (refused by name at birth).
    pub vaspace: Option<u32>,
    /// ★ The guest's own channel id, off `flags` `USERD_INDEX` ([`decode_userd_index_chid`]).
    pub chid: Option<u32>,
    /// `flags` (`NVOS04_FLAGS_*`).
    pub flags: u32,
    /// `engineType`, raw, when the layout is pinned.
    pub engine_type: Option<u32>,
    /// The guest kernel's resolved USERD descriptor (`userdMem`).
    pub userd: Option<kf_arch::UserdMem>,
    /// The client declared the kernel sentinel pid (a guest-KERNEL channel, §7's policy).
    pub kernel_client: bool,
}

/// A statement for the channel plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChanStatement {
    /// A GPFIFO channel alloc.
    Alloc(ChannelAlloc),
    /// `GPFIFO_SCHEDULE` on a channel (`0xa06f0103`) or a TSG (`0xa06c0101`).
    Schedule {
        /// `hClient`.
        client: u32,
        /// The channel or TSG.
        object: u32,
        /// `bEnable`.
        enable: bool,
    },
    /// `BIND` a channel to an engine (`kchannelBindToRunlist`, `kernel_channel.c:2878-2886`).
    Bind {
        /// `hClient`.
        client: u32,
        /// The channel.
        object: u32,
        /// `engineType` (`NV2080_ENGINE_TYPE_*`).
        engine_type: u32,
    },
    /// `GET_WORK_SUBMIT_TOKEN` on a channel.
    Token {
        /// `hClient`.
        client: u32,
        /// The channel.
        object: u32,
    },
    /// An object was freed (maybe one of ours).
    Free {
        /// `hClient`.
        client: u32,
        /// The object (`hObjectOld`).
        object: u32,
    },
}

/// What the plane did with a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChanAnswer {
    /// Not a channel the plane owns: decline (let the chain answer as it did before).
    NotOurs,
    /// Done.
    Done,
    /// Done; the guest's work-submit token.
    Token(u32),
    /// Refused by name, with the NV status the guest reads.
    Refused {
        /// `NV_ERR_*`.
        status: u32,
        /// Why.
        why: String,
    },
}

/// ★ Where statements go. Called on the register drainer (never a vCPU): the plane does its host
/// verbs there, bounded, and answers — a channel's birth is part of the answer to its alloc.
pub type ChanSink = std::sync::Arc<dyn Fn(ChanStatement) -> ChanAnswer + Send + Sync>;

/// ★★★ The link. Seated at the FRONT of the chain (ahead of the object seat, which terminates
/// `GSP_RM_ALLOC`/`GSP_RM_FREE`, and of the ledger, which would record the controls unserviced).
pub struct ChannelPolicy {
    abi: DriverAbiTable,
    guest_os: kf_abi::GuestOs,
    sink: ChanSink,
    /// Clients that declared the kernel sentinel pid.
    kernel_clients: std::collections::BTreeSet<u32>,
    /// `(hClient, parent)` → the first `FERMI_VASPACE_A` allocated under it.
    vas_under: std::collections::BTreeMap<(u32, u32), u32>,
    /// `hClient` → every VA-space object a page-directory statement named in it. ★ The fallback for
    /// `hVASpace = 0` when the VAS alloc itself never reached us: the device-default VAS of an RM
    /// internal client is constructed CPU-side (`vaspaceGetByHandleOrDeviceDefault`), and only its
    /// `COPY_SERVER_RESERVED_PDES` is RPC'd (`[measured p5b]`: no `FERMI_VASPACE_A` alloc seen).
    vas_stated: std::collections::BTreeMap<u32, std::collections::BTreeSet<u32>>,
    /// Statements carried.
    pub carried: u64,
    /// Allocs the plane refused.
    pub refused: u64,
}

impl ChannelPolicy {
    /// A link for one guest driver's wire.
    #[must_use]
    pub fn new(abi: DriverAbiTable, guest_os: kf_abi::GuestOs, sink: ChanSink) -> ChannelPolicy {
        ChannelPolicy { abi, guest_os, sink, kernel_clients: Default::default(), vas_under: Default::default(), vas_stated: Default::default(), carried: 0, refused: 0 }
    }

    fn refusal(status: u32, why: &str, cmd: &RpcCommand) -> Reply {
        eprintln!("kf-rm: channel plane REFUSED ({status:#x}): {why}");
        Reply { rpc_result: status, body: cmd.payload.clone() }
    }

    fn on_alloc(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let body = cmd.wire_body();
        let h = self.abi.decode_rpc_alloc(body).ok()?;
        if self.abi.is_client_root_class(kf_arch::ids::ClassId(h.class)) {
            // Learn the client's kind from the object model's own decode (the sentinel pid).
            if let Ok(crate::rmrpc::Translation::Event(crate::rmgraph::RmEvent::Alloc { facts, .. })) =
                crate::rmrpc::translate(&self.abi, self.guest_os, cmd)
                && matches!(facts.client_kind, Some(kf_arch::ClientKind::Kernel))
            {
                self.kernel_clients.insert(h.handle);
            }
            return None;
        }
        match self.abi.alloc_params(kf_arch::ids::ClassId(h.class)) {
            Some(AllocParams::VaSpace) => {
                let first = *self.vas_under.entry((h.client, h.parent)).or_insert(h.handle);
                eprintln!(
                    "kf-rm: chanlink: FERMI_VASPACE_A {:#x}:{:#x} under {:#x} (device default for it: {first:#x})",
                    h.client, h.handle, h.parent
                );
                return None;
            }
            Some(AllocParams::Channel) => {}
            _ => return None,
        }
        let params = crate::rmrpc::alloc_params_window(&self.abi, body)?;
        let f = self.abi.decode_channel_alloc_facts(params).ok()?;
        let st = ChannelAlloc {
            client: h.client,
            parent: h.parent,
            handle: h.handle,
            class: h.class,
            gpfifo_va: f.gp_fifo_offset,
            entries: f.gp_fifo_entries,
            h_vaspace: f.h_vaspace,
            vaspace: if f.h_vaspace != 0 {
                Some(f.h_vaspace)
            } else {
                // The device's default VAS: allocated under the parent device, or else the ONE
                // VA space this client ever stated a page directory for. Two candidates and no
                // alloc to decide between them is refused by name at birth (`None`).
                self.vas_under.get(&(h.client, h.parent)).copied().or_else(|| {
                    let set = self.vas_stated.get(&h.client)?;
                    (set.len() == 1).then(|| set.iter().next().copied()).flatten()
                })
            },
            chid: decode_userd_index_chid(f.flags),
            flags: f.flags,
            engine_type: self.abi.decode_channel_engine_type(params).ok().flatten(),
            userd: self.abi.decode_channel_userd_mem(params).ok().flatten(),
            kernel_client: self.kernel_clients.contains(&h.client) || is_rm_internal_client(h.client),
        };
        self.carried += 1;
        match (self.sink)(ChanStatement::Alloc(st)) {
            ChanAnswer::Refused { status, why } => {
                self.refused += 1;
                Some(Self::refusal(status, &format!("channel {:#x}:{:#x} alloc: {why}", h.client, h.handle), cmd))
            }
            // Born (or not ours): the object seat records the object and answers.
            _ => None,
        }
    }

    fn on_control(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let h = self.abi.decode_rpc_control(&cmd.payload).ok()?;
        if self.abi.control_params(kf_arch::ids::ControlCmd(h.cmd)).is_some()
            && let Ok(crate::rmrpc::Translation::PageDir(st)) = crate::rmrpc::translate(&self.abi, self.guest_os, cmd)
        {
            // Observed only: the memory plane's link answers these.
            self.vas_stated.entry(st.client.0).or_default().insert(st.vaspace.0);
            return None;
        }
        let params = cmd.payload.get(h.params_at..h.params_at.checked_add(h.params_size as usize)?)?;
        let st = match h.cmd {
            GPFIFO_SCHEDULE | TSG_GPFIFO_SCHEDULE => {
                // `{NvBool bEnable; NvBool bSkipSubmit; NvBool bSkipEnable}` — all [IN].
                ChanStatement::Schedule { client: h.client, object: h.object, enable: params.first().is_some_and(|&b| b != 0) }
            }
            GET_WORK_SUBMIT_TOKEN => ChanStatement::Token { client: h.client, object: h.object },
            BIND => {
                let engine_type = params.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))?;
                ChanStatement::Bind { client: h.client, object: h.object, engine_type }
            }
            _ => return None,
        };
        self.carried += 1;
        match (self.sink)(st) {
            ChanAnswer::NotOurs => None,
            // ★ The [IN] params echoed: the transport copies a non-empty reply over the caller's
            // struct (`ogkm-580: rpc.c:11085-11090`), so a zeroed body would rewrite bEnable.
            ChanAnswer::Done => Some(Reply { rpc_result: NV_OK, body: cmd.payload.clone() }),
            ChanAnswer::Token(t) => {
                let mut body = cmd.payload.clone();
                let at = h.params_at;
                if body.len() < at + 4 || h.params_size < 4 {
                    return Some(Self::refusal(0x1F, "work-submit token params shorter than 4 bytes", cmd));
                }
                body[at..at + 4].copy_from_slice(&t.to_le_bytes());
                Some(Reply { rpc_result: NV_OK, body })
            }
            ChanAnswer::Refused { status, why } => {
                Some(Self::refusal(status, &format!("control {:#010x} on {:#x}:{:#x}: {why}", h.cmd, h.client, h.object), cmd))
            }
        }
    }

    fn on_free(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let f = self.abi.decode_free(&cmd.payload).ok()?;
        let (client, object) = (f.client, f.handle);
        if client == object {
            self.kernel_clients.remove(&client);
            self.vas_under.retain(|k, _| k.0 != client);
            self.vas_stated.remove(&client);
        } else {
            // ★ Only the DEVICE's free forgets its default VAS. The VASpace handle itself is a
            // transient NAME (`index = GPU_DEVICE`, "acquire reference to device vaspace",
            // `nvos.h:3187`): RM allocs it, publishes the PDEs, and FREES it — `[measured p5c]`
            // forgetting it there left the scrubber's channel with no VA space. The graph files it
            // the same way (`RmGraph::device_default_vas`, outliving the handle's own free).
            self.vas_under.retain(|k, _| !(k.0 == client && k.1 == object));
        }
        let _ = (self.sink)(ChanStatement::Free { client, object });
        None
    }
}

/// `RS_CLIENT_INTERNAL_HANDLE_BASE` (`ogkm-580: inc/libraries/resserv/resserv.h:138`).
pub const RS_CLIENT_INTERNAL_HANDLE_BASE: u32 = 0xC1E0_0000;

/// ★ Is `h_client` one of the guest RM's OWN internal clients — the guest's own classification,
/// `serverIsClientInternal` (`ogkm-580: libraries/resserv/src/rs_server.c:2618-2623`).
///
/// `[measured p5a]` the PMA scrubber's client (`0xc1e00006` on that boot) was NOT marked kernel by
/// the root alloc's pid sentinel, so the pid rule alone under-reports RM's internal channels. The
/// handle is the guest RM's own statement: a client that asks for a fixed handle is RE-ENCODED
/// onto the user base `0xC1D00000` (`rs_server.c:3267-3271`), so no guest process can hold one in
/// this range — only the guest kernel's RM makes them.
#[must_use]
pub const fn is_rm_internal_client(h_client: u32) -> bool {
    h_client & RS_CLIENT_INTERNAL_HANDLE_BASE == RS_CLIENT_INTERNAL_HANDLE_BASE
}

/// ★ The guest's own channel id, off `NV_CHANNEL_ALLOC_PARAMS.flags` — the guest kernel allocates
/// its `ChID` before it RPCs and states it as `USERD_INDEX` (`ogkm-580: kernel_channel.c:2786-2800`
/// writes `USERD_INDEX_PAGE_VALUE = chid / 8`, `USERD_INDEX_VALUE = chid % 8`, and sets
/// `USERD_INDEX_PAGE_FIXED`). Fields (`ogkm-580: alloc_channel.h:184-203`): `INDEX_VALUE 10:8`,
/// `INDEX_FIXED 11:11`, `PAGE_VALUE 20:12`, `PAGE_FIXED 21:21`. Copied from the old tree's
/// `kayfabe-chips/src/ga10x.rs:419` (pure; it decoded every boot of the §13 campaign).
///
/// `None` when the page is not fixed (the flags name no channel) or `INDEX_FIXED` is set (RM
/// itself refuses that combination with `NV_ERR_INVALID_STATE`). ★ A USERD page holds
/// `1 << 3` channels, so the widest chid expressible is `511 * 8 + 7 = 4095` — the 12-bit
/// doorbell `VECTOR` exactly.
#[must_use]
pub fn decode_userd_index_chid(flags: u32) -> Option<u32> {
    if (flags >> 21) & 1 == 0 || (flags >> 11) & 1 != 0 {
        return None;
    }
    let value = (flags >> 8) & 0x7;
    let page = (flags >> 12) & 0x1FF;
    Some(page * 8 + value)
}

impl core::fmt::Debug for ChannelPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChannelPolicy").field("carried", &self.carried).field("refused", &self.refused).finish()
    }
}

impl CommandPolicy for ChannelPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        match cmd.function {
            RpcFunction::RmAlloc => self.on_alloc(cmd),
            RpcFunction::RmControl => self.on_control(cmd),
            RpcFunction::Free => self.on_free(cmd),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[cap1b]` the PMA scrubber's channel declared `flags = 0x00a00120` (chid 1); the global
    /// CeUtils rang `0x10002` (chid 2).
    #[test]
    fn the_captured_flags_decode_to_the_guest_chid() {
        assert_eq!(decode_userd_index_chid(0x00a0_0120), Some(1));
        assert_eq!(decode_userd_index_chid(0x00a0_0220), Some(2));
        assert_eq!(decode_userd_index_chid(0x0020_0000 | (511 << 12) | (7 << 8)), Some(4095));
        assert_eq!(decode_userd_index_chid(0x0000_0120), None, "page not fixed: names no channel");
        assert_eq!(decode_userd_index_chid(0x0020_0920), None, "INDEX_FIXED: RM refuses it");
    }

    #[test]
    fn internal_clients_are_the_guest_rms_own() {
        assert!(is_rm_internal_client(0xc1e0_0006));
        assert!(!is_rm_internal_client(0xc1d0_0006), "the user base");
        assert!(!is_rm_internal_client(0xe000_0001), "a VF client");
    }
}
