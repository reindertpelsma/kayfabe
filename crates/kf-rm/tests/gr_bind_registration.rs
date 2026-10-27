//! Production GR bind contracts with bounded fake host effects.
use kf_abi::{
    defapi::{Control, DefApiAbi, Form},
    grbind::{Kind, Request, encode},
};
use kf_rm::defapi::{Bounds, ObjKey, Registry, serve};

fn registration(form: Form, pm: bool) {
    let version = kf_abi::versions::BENCH_DRIVER;
    let abi = DefApiAbi::at(version).unwrap();
    let r = Registry::new(Bounds::default());
    let key = ObjKey {
        client: 0x31,
        object: 0x40,
    };
    let mut buffers = [0; 9];
    buffers[0] = 0x10000;
    let request = Request {
        client: 0x31,
        channel: 0x41,
        kind: if pm {
            Kind::Pm {
                va: 0x10000,
                mode: 1,
            }
        } else {
            Kind::Preemption {
                flags: 2,
                gfxp: 1,
                cilp: 0,
                buffers,
            }
        },
    };
    let (cmd, bytes) = encode(version, request).unwrap();
    let layout = abi.layout(form);
    let mut p = vec![0; layout.size];
    p[layout.handle_off..layout.handle_off + 4].copy_from_slice(&0x40000001u32.to_le_bytes());
    p[layout.cmd_off..layout.cmd_off + 4].copy_from_slice(&cmd.to_le_bytes());
    p[layout.bundle_off..layout.bundle_off + bytes.len()].copy_from_slice(&bytes);
    assert_eq!(
        serve(&r, &abi, key, Control::Register(form), &p, |_| false),
        0
    );
    let entry = r.lookup(key, 0x40000001).unwrap();
    assert_eq!(entry.gr_bind, Some(request));
    // The registration is not a bind completion; the GPU stream must trigger it.
    assert!(!entry.executed);
    assert_eq!(r.clear_life(), 1);
    assert!(r.lookup(key, 0x40000001).is_err());
}
#[test]
fn v1_registration_retains_preemption_intent_at_the_guest_layout() {
    registration(Form::V1, false);
}
#[test]
fn v2_registration_retains_pm_intent_without_executing_a_host_bind() {
    registration(Form::V2, true);
}
