// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
// Read-only inventory of compiled Kayfabe registries. A handler name is not a
// successful replay: object state, payload gates and host facts still matter.
use kf_arch::ids::{ClassId, ControlCmd};
use std::io::{self, BufRead};
fn main() {
    let version = kf_abi::DriverVersion::parse("580.65.06").unwrap();
    let abi = kf_abi::versions::table_for(version).unwrap();
    let display = kf_rm::display::DisplayPolicy::new(*abi, &kf_chip::display::ADA);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (kind, value) = line.split_once('\t').unwrap();
        let id = u32::from_str_radix(value, 16).unwrap();
        let names: Vec<_> = kf_abi::generated::matrix::ALL_VALUES
            .iter()
            .filter(|r| r.at_u32(version).ok().flatten() == Some(id))
            .filter(|r| {
                if kind == "control" {
                    r.name.starts_with("ctrl_limits:") && r.name.contains("_CMD_")
                } else {
                    r.name.starts_with("class_ids:")
                }
            })
            .map(|r| r.name)
            .collect();
        if kind == "control" {
            let wanted = kf_rm::inittables::WantedTable::from_cmd(id);
            let sizes: Vec<_> = kf_abi::gssreplay::ROWS
                .iter()
                .filter(|r| r.cmd == id)
                .map(|r| r.size)
                .collect();
            let rows: Vec<_> = kf_abi::gssreplay::ROWS
                .iter()
                .filter(|r| r.cmd == id)
                .map(|r| {
                    let inputs: Vec<_> =
                        r.inputs.iter().map(|(o, v)| format!("[{o},{v}]")).collect();
                    format!("{{\"size\":{},\"inputs\":[{}]}}", r.size, inputs.join(","))
                })
                .collect();
            use kf_rm::chanlink::*;
            let channel = [
                GPFIFO_SCHEDULE,
                TSG_GPFIFO_SCHEDULE,
                BIND,
                GET_WORK_SUBMIT_TOKEN,
                PROMOTE_CTX,
                EVICT_CTX,
                STOP_CHANNEL,
                DISABLE_CHANNELS,
                TSG_PREEMPT,
                DEBUG_SET_EXCEPTION_MASK,
                GR_SET_CTXSW_PREEMPTION_MODE,
                GR_CTXSW_ZCULL_BIND,
                TSG_SET_TIMESLICE,
                PERF_CUDA_LIMIT_SET_CONTROL,
                PERF_CUDA_LIMIT_DISABLE,
                kf_abi::gssreplay::GSS_ENC_SESSION_ACQUIRE,
                kf_abi::gssreplay::GSS_ENC_SESSION_RELEASE,
            ]
            .contains(&id);
            println!(
                "control\t{id:08x}\t{}\t{:?}\t{:?}\t{:?}\t{}\t{:?}\t{:?}\t{:?}\t{channel}\t[{}]",
                names.join("|"),
                wanted,
                wanted.map(|w| w.params_size()),
                wanted.and_then(|w| w.c_type()),
                display.claims(id),
                abi.capabilities().control(ControlCmd(id)),
                abi.control_params(ControlCmd(id)),
                sizes,
                rows.join(",")
            );
        } else {
            let listed = abi
                .capabilities()
                .all_classes()
                .find(|c| c.class == id)
                .map(|c| c.name)
                .unwrap_or("");
            println!(
                "class\t{id:08x}\t{}\t{listed}\t{:?}\t{:?}\t{:?}\t{}",
                names.join("|"),
                abi.capabilities().alloc_class(ClassId(id)),
                abi.alloc_params(ClassId(id)),
                kf_rm::chanlink::alloc_shape(abi, id),
                kf_rm::display::is_display_class(id)
            );
        }
    }
}
