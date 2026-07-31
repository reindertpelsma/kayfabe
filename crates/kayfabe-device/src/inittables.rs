//! The command policy that answers the two `GSP_RM_CONTROL`s the guest's RM cannot start
//! without, from the chip row's own tables.
//!
//! ## ★★ Why this is here and not in a logic crate
//!
//! It is the composition of two things that already exist: the *rows*
//! ([`crate::ChipProfile::engines`], a fact about silicon) and the *layout*
//! (`kayfabe_abi::inittables`, the Axis-A quarantine). This crate is the adapter where a
//! concrete chip's facts are allowed to meet a wire, so the join belongs here. Nothing in
//! this file names a generation, a driver version or an engine — a second chip is a second
//! row, and this file does not change.
//!
//! ## ★★★ What it does NOT do, deliberately
//!
//! Two controls, both `[OUT]`-only, both answered from a table. It touches no RM graph
//! state, allocates no handle, and remembers nothing between commands. Every other command
//! falls through to whatever the FSM would have done — this is a *supplement* to the
//! baseline policy, not a replacement for `kayfabe_rmrpc::GraphPolicy`, which is the
//! semantic policy the compute path will need.
//!
//! ## ★ It refuses rather than guessing, and the refusal is the loud kind
//!
//! A guest that declares a `paramsSize` other than the one this port's layout produces is
//! a guest whose struct is not the struct we encode. Answering it anyway would hand RM a
//! well-formed table read at the wrong strides — the exact failure mode the `EchoOk` doc
//! argues about at length, where the rejection lives in the payload and nothing logs. So
//! the mismatch is answered with a non-zero **envelope** `rpc_result`, which short-circuits
//! the guest ahead of both the copy-out and the control cache
//! (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:1994`).

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::inittables::{
    self, DEVICE_INFO_PARAMS_SIZE, INTR_PARAMS_SIZE, NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE,
    NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

use crate::ChipProfile;

/// Byte offset of `status` within `rpc_gsp_rm_control_v03_00` — the `[OUT]` field
/// `rpcRmApiControl_GSP` reads *before* it copies params out, and the one that decides
/// whether it copies them at all (`ogkm-580: rpc.c:11061-11065`).
///
/// ★ Not derived from `RpcControlReq`, because that view deliberately omits `status`: it
/// decodes what a guest **sent**, and `status` is a field only a reply fills in.
const CONTROL_STATUS_OFF: usize = 12;

/// Byte offset of `paramsSize` in the same header — rewritten on the reply so RM's
/// copy-out length is ours rather than the request's echo.
const CONTROL_PARAMS_SIZE_OFF: usize = 16;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// Answers the FIFO device-info table and the kernel interrupt table from a chip row.
///
/// Every other command gets `None`, i.e. the FSM's own acknowledgement.
#[derive(Debug, Clone, Copy)]
pub struct InitTablePolicy {
    chip: &'static ChipProfile,
    driver: DriverAbiTable,
}

/// Which of the two tables a command asked for. Returned by [`InitTablePolicy::wanted`] so
/// a test can ask the classification question without building a wire message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WantedTable {
    /// `NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE`.
    DeviceInfo,
    /// `NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE`.
    IntrKernelTable,
}

impl WantedTable {
    /// The `[OUT]` struct size RM allocates for this table.
    #[must_use]
    pub fn params_size(self) -> usize {
        match self {
            Self::DeviceInfo => DEVICE_INFO_PARAMS_SIZE,
            Self::IntrKernelTable => INTR_PARAMS_SIZE,
        }
    }

    /// Classify a control command, or `None` if this policy does not model it.
    #[must_use]
    pub fn from_cmd(cmd: u32) -> Option<WantedTable> {
        match cmd {
            NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE => Some(Self::DeviceInfo),
            NV2080_CTRL_CMD_INTERNAL_INTR_GET_KERNEL_TABLE => Some(Self::IntrKernelTable),
            _ => None,
        }
    }
}

impl InitTablePolicy {
    /// Build the policy for one chip and one guest driver's wire table.
    #[must_use]
    pub fn new(chip: &'static ChipProfile, driver: DriverAbiTable) -> InitTablePolicy {
        InitTablePolicy { chip, driver }
    }

    /// Which table this command asks for, if any — the classification step on its own.
    #[must_use]
    pub fn wanted(&self, cmd: &RpcCommand) -> Option<WantedTable> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        WantedTable::from_cmd(req.cmd)
    }
}

/// A reply that carries no body and a non-zero envelope result — the short-circuit.
fn refuse() -> Option<Reply> {
    Some(Reply {
        rpc_result: NV_ERR_NOT_SUPPORTED,
        body: Vec::new(),
    })
}

impl CommandPolicy for InitTablePolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::RmControl {
            return None;
        }
        // A payload too short to hold the control header is not a control this policy can
        // even classify; leave it to the baseline rather than inventing a refusal for a
        // message that may not be one.
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let want = WantedTable::from_cmd(req.cmd)?;

        // A FINN-serialized payload is not the flat struct these encoders produce. Neither
        // control appears serialized anywhere this port has looked — the C answers both
        // flat and a real driver accepted it — but that is an absence of observation, not
        // a guarantee, and an unchecked flat answer is the kind of wrong that never logs.
        if kayfabe_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {
            return refuse();
        }
        // The guest's own declared size must be the size we encode, and its payload must
        // actually hold it. Both are the guest's assertions, so both are checked.
        if req.params_size as usize != want.params_size()
            || cmd.payload.len() < req.params_at + want.params_size()
        {
            return refuse();
        }

        let params = match want {
            WantedTable::DeviceInfo => {
                // `baseIndex` is the guest's paging cursor, at the head of its own params.
                let at = req.params_at;
                let base_index = u32::from_le_bytes([
                    cmd.payload[at],
                    cmd.payload[at + 1],
                    cmd.payload[at + 2],
                    cmd.payload[at + 3],
                ]);
                match inittables::encode_device_info_table(self.chip.engines, base_index) {
                    Ok(p) => p.params,
                    Err(_) => return refuse(),
                }
            }
            WantedTable::IntrKernelTable => match inittables::encode_intr_kernel_table(
                self.chip.intr_table,
                &self.chip.intr_subtree_map,
            ) {
                Ok(p) => p,
                Err(_) => return refuse(),
            },
        };

        // Keep the guest's own control header — `hClient`/`hObject`/`cmd` are echoed, as
        // they are on every real reply — and overwrite only the two fields a GSP owns.
        let mut body = cmd.payload.clone();
        body[CONTROL_STATUS_OFF..CONTROL_STATUS_OFF + 4].copy_from_slice(&NV_OK.to_le_bytes());
        let size = u32::try_from(params.len()).unwrap_or(u32::MAX);
        body[CONTROL_PARAMS_SIZE_OFF..CONTROL_PARAMS_SIZE_OFF + 4]
            .copy_from_slice(&size.to_le_bytes());
        body[req.params_at..req.params_at + params.len()].copy_from_slice(&params);

        Some(Reply {
            rpc_result: NV_OK,
            body,
        })
    }
}

kayfabe_util::assert_send_sync!(InitTablePolicy, WantedTable);
