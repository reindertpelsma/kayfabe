//! ★ 2026-10-07 (Windows Code43): the host-owned stub — `NV_OK` with zeroed params for the
//! generated set of [`kf_abi::hoststub`], for a guest whose declared identity is the cell's
//! Windows build. Owner ruling §S as **ASSUMED from the owner's 2026-10-07 statement that
//! privileged non-compute actions can be stubbed; owner to confirm** (`docs/OWNER_RULINGS.md` §S).
//!
//! Why this one control and nothing else: the bisect of runs 38-41
//! (`traces/windows_code43_walls_20261007/README.md`, "Run41 result") measured that answering
//! `0x2081010d` alone moves Windows' StartDevice past VFIO 2861, and that neither of the other two
//! late controls alone does, nor do the ten early ones. The control has no public name, layout or
//! body; the guest declares zero params bytes for it (measured, `kf_abi::hoststub`), so a
//! zero-filled answer carries no invented value — only the status. That status is the stub.
//!
//! ★ 2026-10-31 (provenance fix): the answer line no longer prints the control's export flags
//! (a value that came from a closed binary and that nothing branched on); the cell no longer
//! carries it. The answer rule is unchanged: `NV_OK` only for the cell's id at exactly its
//! declared params size, never for a serialized envelope.
//!
//! ⊘ What it never does: forward anything to the host, touch GPU state, answer a control outside
//! the generated cell, answer before the guest declared the cell's identity, answer a serialized
//! (FINN) envelope, or answer a declared params size other than the generated one. It replaces the
//! default-off `KF3_DIAG_ZERO_OK` diagnostic of runs 37-41, which is removed.

use kf_abi::hoststub;
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

const NV_OK: u32 = 0;
/// Answers logged per run (the rest are counted only).
const LOG_CAP: u64 = 16;

/// The stub link.
#[derive(Debug, Clone)]
pub struct HostStubPolicy {
    driver: kf_abi::versions::DriverAbiTable,
    cell: Option<&'static hoststub::StubCell>,
    identity: bool,
    answered: u64,
}

impl HostStubPolicy {
    /// A link for the guest driver `driver`. Inert until a matching identity is declared.
    #[must_use]
    pub fn new(driver: kf_abi::versions::DriverAbiTable) -> HostStubPolicy {
        HostStubPolicy {
            driver,
            cell: hoststub::cell(driver.driver_version()),
            identity: false,
            answered: 0,
        }
    }

    /// Answers given so far.
    #[must_use]
    pub fn answered(&self) -> u64 {
        self.answered
    }
}

impl CommandPolicy for HostStubPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let cell = self.cell?;
        if cmd.function == RpcFunction::SetGuestSystemInfo {
            // Every new declaration replaces the old one; a changed or malformed one revokes.
            self.identity = cell.matches_identity(&cmd.payload)
                && kf_abi::guestsysinfo::decode_declared_vgx(&cmd.payload).ok()
                    == self.driver.vgx_version();
            return None;
        }
        if cmd.function != RpcFunction::RmControl || !self.identity {
            return None;
        }
        let req = self.driver.decode_rpc_control(&cmd.payload).ok()?;
        let stub = cell.control(req.cmd)?;
        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags)
            || req.params_size as usize != stub.param_size
        {
            return None;
        }
        let end = req.params_at.checked_add(stub.param_size)?;
        if end > cmd.payload.len() {
            return None;
        }
        let mut body = cmd.payload.clone();
        let st = self.driver.rm_control_wire().status_off;
        body.get_mut(st..st + 4)?
            .copy_from_slice(&NV_OK.to_le_bytes());
        body[req.params_at..end].fill(0);
        self.answered += 1;
        if self.answered <= LOG_CAP {
            eprintln!(
                "kf-rm: HOST-STUB {:#010x} ({} msg {:#x}) client={:#x}: NV_OK, {} params byte(s) zeroed — host-owned stub, §S assumed #{}",
                req.cmd, stub.interface, stub.message, req.client, stub.param_size, self.answered
            );
        }
        Some(Reply {
            rpc_result_private: None,
            rpc_result: NV_OK,
            body,
        })
    }
}

kf_util::assert_send_sync!(HostStubPolicy);

#[cfg(test)]
mod tests {
    use super::*;

    fn abi() -> kf_abi::versions::DriverAbiTable {
        *kf_abi::versions::table_for(kf_abi::versions::BENCH_DRIVER).expect("bench")
    }

    fn windows_abi() -> Option<kf_abi::versions::DriverAbiTable> {
        kf_abi::versions::table_for(kf_abi::DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .ok()
        .copied()
    }

    fn control(
        abi: &kf_abi::versions::DriverAbiTable,
        cmd: u32,
        params: &[u8],
        flags: u32,
    ) -> RpcCommand {
        let w = abi.rm_control_wire();
        let mut payload = vec![0u8; w.params_off + params.len()];
        payload[0..4].copy_from_slice(&0xc1d0_0002u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0xff0d_0000u32.to_le_bytes());
        payload[8..12].copy_from_slice(&cmd.to_le_bytes());
        payload[w.status_off..w.status_off + 4].copy_from_slice(&0x56u32.to_le_bytes());
        payload[w.params_size_off..w.params_size_off + 4]
            .copy_from_slice(&u32::try_from(params.len()).unwrap().to_le_bytes());
        payload[w.rpc_flags_off..w.rpc_flags_off + 4].copy_from_slice(&flags.to_le_bytes());
        payload[w.params_off..].copy_from_slice(params);
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0x4c,
            sequence: 1,
            payload,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    /// A `SET_GUEST_SYSTEM_INFO` declaring Windows 580.88 at `w` (the `sw_runlist_probe` fixture).
    fn identity(w: &kf_abi::versions::DriverAbiTable, name: &str) -> RpcCommand {
        use kf_abi::guestsysinfo::*;
        let twin = kf_abi::generated::windows_twins::WINDOWS_TWINS
            .iter()
            .find(|t| t.win_name == "580.88")
            .unwrap();
        let mut p = vec![0; SET_GUEST_SYSTEM_INFO_SIZE];
        let vgx = w.vgx_version().unwrap();
        p[VGX_MAJOR_OFF..VGX_MAJOR_OFF + 4].copy_from_slice(&vgx.major.to_le_bytes());
        p[VGX_MINOR_OFF..VGX_MINOR_OFF + 4].copy_from_slice(&vgx.minor.to_le_bytes());
        p[GUEST_DRIVER_VERSION_OFF..GUEST_DRIVER_VERSION_OFF + name.len()]
            .copy_from_slice(name.as_bytes());
        p[GUEST_VERSION_OFF..GUEST_VERSION_OFF + twin.win_branch.len()]
            .copy_from_slice(twin.win_branch.as_bytes());
        p[GUEST_CL_NUM_OFF..GUEST_CL_NUM_OFF + 4].copy_from_slice(&twin.win_cl.to_le_bytes());
        RpcCommand {
            function: RpcFunction::SetGuestSystemInfo,
            code: 0,
            sequence: 1,
            payload: p,
            elements: 1,
            delivered: Vec::new(),
        }
    }

    #[test]
    fn a_declared_identity_arms_the_cell_and_a_changed_one_revokes_it() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let mut p = HostStubPolicy::new(w);
        assert!(
            p.respond(&identity(&w, "580.88")).is_none(),
            "fn1 is observed, never answered"
        );
        assert!(p.respond(&control(&w, 0x2081_010d, &[], 0)).is_some());
        p.respond(&identity(&w, "999.99"));
        assert!(p.respond(&control(&w, 0x2081_010d, &[], 0)).is_none());
    }

    /// ★ 2026-10-31 (provenance fix): the Windows-cell source tags are the published constant
    /// table's own grammar (`consttab`): the same parser accepts every one of them.
    #[test]
    fn the_windows_cell_tags_parse_under_the_constant_table_grammar() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let v = w.driver_version();
        let stub = kf_abi::hoststub::cell(v).expect("stub cell");
        let rl = kf_abi::sw_runlist::cell(v).expect("runlist cell");
        let mut n = 0;
        for row in stub
            .controls
            .iter()
            .flat_map(|c| c.rows.iter())
            .chain(rl.rows.iter())
        {
            crate::consttab::parse_source(row.source)
                .unwrap_or_else(|e| panic!("{}: {e}", row.item));
            n += 1;
        }
        assert!(n >= 8, "only {n} rows");
    }

    /// ★ 2026-10-31: the answer rule is exactly the one the generated table gave - the declared
    /// size must equal the cell's `param_size`, the id must be the cell's, and the reply is
    /// `NV_OK` with the params zeroed; nothing reads the dropped export fields.
    #[test]
    fn the_answer_rule_is_the_cells_id_and_exact_declared_size() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let cell = kf_abi::hoststub::cell(w.driver_version()).expect("stub cell");
        assert_eq!(cell.controls.len(), 1);
        let s = &cell.controls[0];
        let mut p = HostStubPolicy::new(w);
        p.identity = true;
        for size in 0..=8usize {
            let params = vec![0xa5u8; size];
            let r = p.respond(&control(&w, s.cmd, &params, 0));
            assert_eq!(r.is_some(), size == s.param_size, "declared size {size}");
        }
    }

    #[test]
    fn a_driver_without_a_cell_answers_nothing() {
        // The bench driver (580.159.04) has no Windows cell.
        let mut p = HostStubPolicy::new(abi());
        assert!(p.cell.is_none());
        assert!(p.respond(&control(&abi(), 0x2081_010d, &[], 0)).is_none());
    }

    #[test]
    fn without_a_declared_identity_the_cell_is_inert() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let mut p = HostStubPolicy::new(w);
        assert!(p.cell.is_some(), "580.65.06 has the Windows 580.88 cell");
        assert!(p.respond(&control(&w, 0x2081_010d, &[], 0)).is_none());
        assert_eq!(p.answered(), 0);
    }

    #[test]
    fn with_the_identity_only_the_generated_control_at_its_size_is_answered() {
        let w = windows_abi().expect("580.65.06 has a wire table");
        let mut p = HostStubPolicy::new(w);
        p.identity = true;
        let r = p
            .respond(&control(&w, 0x2081_010d, &[], 0))
            .expect("the stub answers");
        assert_eq!(r.rpc_result, NV_OK);
        let st = w.rm_control_wire().status_off;
        assert_eq!(&r.body[st..st + 4], &[0; 4]);
        // Wrong declared size, a serialized envelope, another control, another function: no.
        assert!(p.respond(&control(&w, 0x2081_010d, &[1; 4], 0)).is_none());
        assert!(
            p.respond(&control(&w, 0x2081_010d, &[], 0xffff_ffff))
                .is_none()
        );
        assert!(p.respond(&control(&w, 0x2080_a801, &[], 0)).is_none());
        let mut free = control(&w, 0x2081_010d, &[], 0);
        free.function = RpcFunction::Free;
        assert!(p.respond(&free).is_none());
        assert_eq!(p.answered(), 1);
    }
}
