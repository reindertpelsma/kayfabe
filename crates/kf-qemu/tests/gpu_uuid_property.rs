//! The `gpu-uuid` / `vm-id` device properties, GPU-free. Malformed values are refused BY NAME at
//! `Device::realize` before anything is opened (same as `display-max-fps`: a realize that skipped
//! `Config::check` would go on to open `/dev` and fail on something else); the resolution is
//! `kf_qemu::gpuuid::resolve_for`, the function realize calls.

use kf_abi::gspstaticinfo::GpuGid;
use kf_qemu::device::{Config, Device};
use kf_qemu::gpuuid::resolve_for;
use kf_rm::gpuuid::Basis;

fn config(gpu_uuid: Option<&str>, vm_id: Option<&str>, devfn: u32) -> Config {
    Config {
        gpu_minor: 0,
        fb_mb: 64,
        bar1_bytes: 256 << 20,
        bar2_bytes: 32 << 20,
        guest_driver: None,
        display: false,
        x11_dispsw: false,
        display_broker: false,
        display_broker_vram: kf_broker::gpucopy::VramMode::default(),
        gop: false,
        gop_efi: None,
        display_max_fps: 0,
        gpu_uuid: gpu_uuid.map(str::to_owned),
        vm_id: vm_id.map(str::to_owned),
        pci_devfn: devfn,
        channel_budget: 0,
    }
}

fn host() -> Option<GpuGid> {
    GpuGid::parse("GPU-51b08678-7828-4015-1962-a65a7a488e3c").ok()
}

const VM1: &str = "11111111-1111-1111-1111-111111111111";
const VM2: &str = "22222222-2222-2222-2222-222222222222";

fn no_entropy(_: &mut [u8; 16]) -> Result<(), String> {
    Err("entropy must not be asked".into())
}

#[test]
fn realize_refuses_hostile_properties_by_name_before_opening_anything() {
    let long = "f".repeat(4096);
    for (c, word) in [
        (config(Some("GPU-nope"), None, 0), "gpu-uuid="),
        (
            config(Some("0000000000000000-0000-0000000000000000"), None, 0),
            "gpu-uuid=",
        ),
        (
            config(Some("GPU-00000000-0000-0000-0000-000000000000"), None, 0),
            "all-zero",
        ),
        (config(Some("random; rm -rf /"), None, 0), "gpu-uuid="),
        (config(Some(&long), None, 0), "gpu-uuid="),
        (config(None, Some("not-a-uuid"), 0), "vm-id="),
        (config(None, Some(&long), 0), "vm-id="),
        (config(None, None, 256), "devfn"),
    ] {
        match Device::realize(&c) {
            Ok(_) => panic!("realize accepted {c:?}"),
            Err(e) => {
                assert!(e.contains(word), "wanted {word:?} in {e:?}");
                assert!(e.len() < 500, "bounded: {}", e.len());
            }
        }
    }
}

#[test]
fn good_properties_pass_check() {
    for c in [
        config(None, None, 0),
        config(Some("auto"), Some(VM1), 0x18),
        config(Some("random"), None, 255),
        config(Some("host"), None, 0),
        config(Some("GPU-51b08678-7828-4015-1962-a65a7a488e3c"), None, 0),
    ] {
        assert_eq!(c.check(), Ok(()), "{c:?}");
    }
}

/// Two devices on one chip row (and one host GPU) in two VMs differ; the same VM is stable.
#[test]
fn different_vm_ids_differ_and_the_same_vm_id_is_stable() {
    let uuid = |vm: &str, devfn| {
        resolve_for(&config(None, Some(vm), devfn), host(), &mut no_entropy)
            .expect("resolves")
            .gid
    };
    assert_ne!(uuid(VM1, 0x18), uuid(VM2, 0x18));
    assert_eq!(uuid(VM1, 0x18), uuid(VM1, 0x18));
    // Two kf3 devices in ONE VM on ONE host GPU (they share vm id and host UUID): the slot
    // separates them.
    assert_ne!(uuid(VM1, 0x18), uuid(VM1, 0x20));
    // `auto` spelled out is the default.
    let spelled = resolve_for(
        &config(Some("auto"), Some(VM1), 0x18),
        host(),
        &mut no_entropy,
    );
    assert_eq!(spelled.unwrap().gid, uuid(VM1, 0x18));
}

#[test]
fn random_differs_between_boots_with_the_real_entropy_source() {
    let boot = || {
        resolve_for(
            &config(Some("random"), Some(VM1), 0),
            None,
            &mut kf_rm::gpuuid::os_entropy,
        )
        .expect("resolves")
    };
    let (a, b) = (boot(), boot());
    assert_eq!((a.basis, b.basis), (Basis::Random, Basis::Random));
    assert_ne!(a.gid, b.gid);
}

#[test]
fn explicit_round_trips_and_host_warns() {
    let text = "GPU-51b08678-7828-4015-1962-a65a7a488e3c";
    let r = resolve_for(&config(Some(text), None, 0), None, &mut no_entropy).unwrap();
    assert_eq!(r.gid.to_string(), text);
    let r = resolve_for(&config(Some("host"), Some(VM1), 0), host(), &mut no_entropy).unwrap();
    assert_eq!(Some(r.gid), host());
    assert_eq!(r.warnings.len(), 1);
}

/// The default (no property at all, QEMU's `-uuid` unset) must neither refuse the VM nor collide:
/// it is a random UUID with a named warning.
#[test]
fn a_vm_with_no_identity_still_starts_with_a_named_random_uuid() {
    let mut n = 0u8;
    let mut src = |b: &mut [u8; 16]| {
        n += 1;
        *b = [n; 16];
        Ok(())
    };
    let c = config(None, None, 0);
    let (a, b) = (
        resolve_for(&c, host(), &mut src).unwrap(),
        resolve_for(&c, host(), &mut src).unwrap(),
    );
    assert_eq!(a.basis, Basis::RandomNoVmId);
    assert_ne!(a.gid, b.gid);
    assert!(a.warnings[0].contains("-uuid"));
    // QEMU's unset -uuid is all-zero text: the same thing.
    let zero = config(None, Some("00000000-0000-0000-0000-000000000000"), 0);
    assert_eq!(
        resolve_for(&zero, host(), &mut src).unwrap().basis,
        Basis::RandomNoVmId
    );
}
