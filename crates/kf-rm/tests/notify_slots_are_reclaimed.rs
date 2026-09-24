//! ★★★★★ **w760p — the notify-slot leak that WAS the device-open wall.**
//!
//! `[measured w760, RMLADDER_OPEN_PROBE=8, guest on 580.159.04]` opens 1–4 succeed, open 5
//! HANGS, opens 6–8 fail fast, and the device is unopenable for the rest of that QEMU's life.
//! The head of that chain is here, and the arithmetic is exact:
//!
//! - every device open arms **three** notifiers on three fresh `(hClient, hObject)` pairs —
//!   `POWER_RESUME` (194) on the subdevice, and `FIFO_EVENT_MTHD` (35) twice;
//! - [`NOTIFY_SUBDEVICE_SLOTS`] is **16**, and a slot was released only by an explicit
//!   `ACTION_DISABLE`, never by a `FREE`;
//! - a closing guest driver frees its **client**, it does not disarm notifier by notifier;
//! - ⇒ 5 opens × 3 = 15 slots, the 6th open's `194` takes the 16th, and its two index-35
//!   armings find no free slot. The census recorded exactly that: `16 served + 2 REFUSED`.
//!
//! And a refused index-35 arming is not a small thing: it fails
//! `_memmgrMemUtilsScrubInitRegisterCallback` ⇒ `scrubberConstruct` ⇒ the heap is never built
//! ⇒ `memmgrGetDeviceSuballocator` returns NULL ⇒ `kbusInitBar2` ⇒ `RmInitAdapter` fails
//! **before** `NV_INIT_FLAG_GPU_STATE_LOAD` is set ⇒ its unwind skips `gpuStateUnload` ⇒
//! Booter Unload never runs ⇒ WPR2 stays latched forever (`THE_CONSTRAINTS.md` §40).
//!
//! ⊘ The leak was *named in the source* and dismissed with *"The boot path frees no armed
//! subdevice"* — true of one boot, false of every open after it. That sentence is the defect.

#[path = "support/ga106.rs"]
mod ga106;

use kf_abi::eventnotify::{
    ACTION_REPEAT, EVENT_SET_NOTIFICATION_PARAMS_SIZE, EventSetNotification,
    NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION,
};
use kf_abi::versions::{BENCH_DRIVER, table_for};
use kf_rm::inittables::{InitTablePolicy, NOTIFY_SUBDEVICE_SLOTS};
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};

const PARAMS_AT: usize = 40;

/// `NV2080_NOTIFIERS_FIFO_EVENT_MTHD` — `cl2080_notification.h:72`. The scrubber's notifier,
/// and the one the wall actually refused.
const FIFO_EVENT_MTHD: u32 = 35;

fn policy() -> InitTablePolicy {
    InitTablePolicy::new(ga106::board(), ga106::host(), *table_for(BENCH_DRIVER).expect("bench ABI"))
}

/// The scrubber's own registration — index 35, `REPEAT`.
fn arm(client: u32, object: u32) -> RpcCommand {
    let reg = EventSetNotification {
        event: FIFO_EVENT_MTHD,
        action: ACTION_REPEAT,
        notify_state: false,
        info32: 0,
        info16: 0,
    };
    let size = u32::try_from(EVENT_SET_NOTIFICATION_PARAMS_SIZE).unwrap();
    let mut payload = vec![0u8; PARAMS_AT + size as usize];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[4..8].copy_from_slice(&object.to_le_bytes());
    payload[8..12].copy_from_slice(&NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION.to_le_bytes());
    payload[16..20].copy_from_slice(&size.to_le_bytes());
    payload[PARAMS_AT..PARAMS_AT + 4].copy_from_slice(&reg.event.to_le_bytes());
    payload[PARAMS_AT + 4..PARAMS_AT + 8].copy_from_slice(&reg.action.to_le_bytes());
    RpcCommand {
        function: RpcFunction::RmControl,
        code: 0x4c,
        sequence: 25,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

/// `NVOS00_PARAMETERS`: `hRoot` @0, `hObjectParent` @4, `hObjectOld` @8. A driver closing its
/// device frees the CLIENT, so `hObjectOld == hRoot` — the shape that matters.
fn free_client(client: u32) -> RpcCommand {
    let mut payload = vec![0u8; 16];
    payload[0..4].copy_from_slice(&client.to_le_bytes());
    payload[8..12].copy_from_slice(&client.to_le_bytes());
    RpcCommand {
        function: RpcFunction::Free,
        code: 0x0a,
        sequence: 26,
        payload,
        elements: 1,
        delivered: Vec::new(),
    }
}

fn served(p: &mut InitTablePolicy, cmd: &RpcCommand) -> bool {
    matches!(p.respond(cmd), Some(r) if r.rpc_result == 0)
}

/// The CONTROL, and the bound restated: 16 slots, and the 17th distinct subdevice is refused.
/// ⊘ Without this, "the 17th succeeded" could pass for a reclaim that silently evicted.
#[test]
fn the_bound_is_still_sixteen_and_still_fails_loud() {
    let mut p = policy();
    for i in 0..NOTIFY_SUBDEVICE_SLOTS {
        let c = 0xc1e0_0000 + u32::try_from(i).unwrap();
        assert!(served(&mut p, &arm(c, 0xb)), "slot {i} should be free");
    }
    let over = 0xc1e0_0000 + u32::try_from(NOTIFY_SUBDEVICE_SLOTS).unwrap();
    assert!(
        !served(&mut p, &arm(over, 0xb)),
        "★ the {}th distinct subdevice must be REFUSED — bounded state fails loud, and a \
         reclaim must never turn that into a silent eviction",
        NOTIFY_SUBDEVICE_SLOTS + 1
    );
}

/// ★★★★★ **THE FIX.** Freeing the client releases its slots, so the table recovers.
#[test]
fn freeing_the_client_releases_its_slots() {
    let mut p = policy();
    for i in 0..NOTIFY_SUBDEVICE_SLOTS {
        let c = 0xc1e0_0000 + u32::try_from(i).unwrap();
        assert!(served(&mut p, &arm(c, 0xb)));
    }
    // Full: the next distinct subdevice is refused.
    let fresh = 0xc1e0_1000;
    assert!(!served(&mut p, &arm(fresh, 0xb)), "precondition: the table is full");

    p.respond(&free_client(0xc1e0_0000));

    assert!(
        served(&mut p, &arm(fresh, 0xb)),
        "★★★★★ a freed client's slot was NOT reclaimed. Every device open leaks three of \
         these, so the 6th open's scrubber arming is refused and the GPU is unopenable for \
         the rest of the VM's life."
    );
}

/// ★★★★★ **THE PROPERTY THE OWNER ASKED FOR: cycling load/unload just works.**
///
/// Twenty open/close cycles at three armings each — 60 armings through a 16-slot table.
/// Before w760p this failed on cycle 6, which is exactly where the hardware wall sat.
#[test]
fn twenty_open_close_cycles_never_exhaust_the_table() {
    let mut p = policy();
    for cycle in 0..20u32 {
        let base = 0xc1e0_0000 + cycle * 0x10;
        // What one device open actually arms: the subdevice, then two scrubber channels.
        for (n, obj) in [(base, 0xcaf0_0001u32), (base + 1, 0xb), (base + 2, 0xc)] {
            assert!(
                served(&mut p, &arm(n, obj)),
                "★ cycle {cycle} could not arm — the table ran out. A guest that opens \
                 /dev/nvidia0 a few times and then never again is broken for anything real: \
                 a CUDA process that restarts, a container runtime, a second tenant."
            );
        }
        // Close: the driver frees its client, not each notifier.
        for n in [base, base + 1, base + 2] {
            p.respond(&free_client(n));
        }
    }
}
