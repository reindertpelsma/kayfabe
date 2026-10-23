//! CE/SA-02 boundary replay. FALSIFIER: the census birth changes route, a guest
//! object widens the host seed, or a Windows USER/ADMIN stamp authorizes C1E.
use kf_abi::guestsysinfo::*;
use kf_gsp::{CommandPolicy, RpcCommand, RpcFunction};
use kf_rm::chanlink::{ChanAnswer, ChanSink, ChanStatement, ChannelAlloc, ChannelPolicy};
use std::sync::{Arc, Mutex};

fn command(function: RpcFunction, payload: Vec<u8>) -> RpcCommand {
    RpcCommand {
        function,
        code: 103,
        sequence: 1,
        payload,
        elements: 1,
        delivered: vec![],
    }
}
fn alloc(client: u32, parent: u32, class: u32, object: u32, params: Vec<u8>) -> RpcCommand {
    let mut payload: Vec<u8> = [client, parent, object, class, 0, params.len() as u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    payload.extend(params);
    command(RpcFunction::RmAlloc, payload)
}
fn policy() -> (ChannelPolicy, Arc<Mutex<Vec<ChanStatement>>>) {
    let abi = *kf_abi::versions::table_for(kf_abi::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    })
    .unwrap();
    let seen = Arc::new(Mutex::new(vec![]));
    let out = seen.clone();
    let sink: ChanSink = Arc::new(move |st| {
        out.lock().unwrap().push(st);
        ChanAnswer::Done
    });
    let mut link =
        ChannelPolicy::new(abi, kf_abi::GuestOs::Windows, sink).with_user_work_flag(true);
    let twin = kf_abi::generated::windows_twins::WINDOWS_TWINS
        .iter()
        .find(|t| t.win_name == "580.88")
        .unwrap();
    let mut identity = vec![0; SET_GUEST_SYSTEM_INFO_SIZE];
    let vgx = abi.vgx_version().unwrap();
    for (off, value) in [
        (VGX_MAJOR_OFF, vgx.major),
        (VGX_MINOR_OFF, vgx.minor),
        (GUEST_CL_NUM_OFF, twin.win_cl),
    ] {
        identity[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (off, value) in [
        (GUEST_DRIVER_VERSION_OFF, twin.win_name),
        (GUEST_VERSION_OFF, twin.win_branch),
    ] {
        identity[off..off + value.len()].copy_from_slice(value.as_bytes());
    }
    assert!(
        kf_abi::hoststub::cell(abi.driver_version())
            .unwrap()
            .matches_identity(&identity)
    );
    let _ = link.respond(&command(RpcFunction::SetGuestSystemInfo, identity));
    (link, seen)
}
fn birth(client: u32, stamp: u32) -> (ChannelPolicy, Arc<Mutex<Vec<ChanStatement>>>, ChannelAlloc) {
    let (mut link, seen) = policy();
    let root = [client, 0x310]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let _ = link.respond(&alloc(client, client, 0, 0, root));
    let version = kf_abi::DriverVersion {
        major: 580,
        minor: 65,
        patch: 6,
    };
    let layout =
        kf_abi::matrix::Resolved::of(&kf_abi::generated::matrix::NV_CHANNEL_ALLOC_PARAMS, version)
            .unwrap();
    let mut params = vec![0; layout.size()];
    for (field, value) in [
        ("internalFlags", stamp),
        ("ProcessID", 0x310),
        ("flags", 0x200620),
        ("gpFifoEntries", 64),
        ("engineType", 0xb),
        ("hContextShare", 0),
    ] {
        let at = layout.need(field).unwrap().off();
        params[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let _ = link.respond(&alloc(client, 0xff0e0000, 0xc56f, 0xff040004, params));
    let a = seen
        .lock()
        .unwrap()
        .iter()
        .find_map(|s| match s {
            ChanStatement::Alloc(a) => Some(*a),
            _ => None,
        })
        .expect("real channel allocation");
    (link, seen, a)
}

#[test]
fn census_kernel_copy2_birth_keeps_its_translated_route() {
    let (_, _, a) = birth(0xc1d00019, 0x16);
    assert!(a.kernel_client);
    assert!(!a.user_work);
    assert!(a.user_ring_permit.is_none());
    assert_eq!(a.engine_type, Some(0xb));
    assert_eq!(a.process_id, Some(0x310));
    assert_eq!(a.parent, 0xff0e0000);
    assert_eq!(a.flags, 0x200620);
}
#[test]
fn windows_c1e_user_stamp_still_denies_kernel_authority() {
    let (_, _, a) = birth(0xc1e00100, 0);
    assert!(!a.kernel_client);
    assert!(a.user_ring_permit.is_none());
}
#[test]
fn windows_c1e_admin_stamp_still_denies_kernel_authority() {
    let (_, _, a) = birth(0xc1e00100, 1);
    assert!(!a.kernel_client);
    assert!(a.user_ring_permit.is_none());
}

fn guest_object(class: u32) {
    let (mut link, seen, a) = birth(0xc1d00019, 0x16);
    let mut params = vec![0; kf_abi::submit::CeAllocParams::SIZE];
    kf_abi::submit::CeAllocParams {
        version: 1,
        engine_type: 0xb,
    }
    .encode_into(&mut params)
    .unwrap();
    let _ = link.respond(&alloc(a.client, a.handle, class, 0x42, params));
    assert!(seen.lock().unwrap().iter().any(|st| matches!(st,
        ChanStatement::EngineObject { parent, class: c, copy_engine: Some(0xb), .. }
            if *parent == a.handle && *c == class)));
    let host = HostFacts {
        ce: 0xc7b5,
        video: 0,
    };
    let config = kf_qemu::chan::translated_decoder_config(a.engine_type.unwrap(), &host, false);
    assert_eq!(kf_qemu::chan::ce_seed_log(config), "ce_seed=0xc7b5");
    let mut ring = kf_chan::ring::TranslatedRing::new_tmode(0x1000, 64, 0);
    ring.set_gr(config);
    struct Mem;
    impl kf_chan::ring::GuestMemory for Mem {
        fn read(&mut self, va: u64, out: &mut [u8]) -> Result<(), String> {
            let words = [0x20038090u32, 1, 0x2050_f060, 1, 0x200180c0, 0x4c];
            let data = if va == 0x1000 {
                kf_abi::submit::gp_entry(0x2000, 24)
                    .unwrap()
                    .to_le_bytes()
                    .to_vec()
            } else if va == 0x2000 {
                words.into_iter().flat_map(u32::to_le_bytes).collect()
            } else {
                return Err("unexpected test VA".into());
            };
            assert_eq!(out.len(), data.len());
            out.copy_from_slice(&data);
            Ok(())
        }
    }
    struct W;
    impl kf_chan::translated::Window for W {
        fn translate(&self, _: kf_chan::translated::Target, _: u64, _: u64) -> Option<u64> {
            panic!("unbound only")
        }
    }
    let next = ring
        .next(1, &mut Mem, kf_chip::is_any_dma_copy_class, &W)
        .expect("guest object does not replace host CE seed");
    assert!(matches!(next, kf_chan::ring::Next::Bind { ref ir, .. }
        if matches!(&ir[..], [kf_chan::tmode::Ir::Launch(l)] if l.tier == kf_chan::ttables::Tier::C7b5)));
}
#[test]
fn a_guest_c8b5_object_does_not_select_the_birth_tier() {
    guest_object(0xc8b5);
}
#[test]
fn a_guest_c9b5_object_does_not_select_the_birth_tier() {
    guest_object(0xc9b5);
}

struct HostFacts {
    ce: u32,
    video: u32,
}
impl kf_qemu::chan::HostRingFacts for HostFacts {
    fn ce_class(&self) -> u32 {
        self.ce
    }
    fn video_context(&self) -> Option<(u32, u32, u32)> {
        (self.video != 0).then_some((0x42, self.video, 0x13))
    }
}
#[test]
fn birth_logging_and_wiring_use_another_host_ce_class() {
    let host = HostFacts {
        ce: 0xc6b5,
        video: 0,
    };
    let cfg = kf_qemu::chan::translated_decoder_config(0xb, &host, false);
    assert_eq!(cfg.ce, host.ce);
    assert_eq!(kf_qemu::chan::ce_seed_log(cfg), "ce_seed=0xc6b5");
}
#[test]
fn graphics_and_video_births_have_no_copy_seed() {
    for (engine, video, tier) in [(1, 0, true), (0xb, 0xc9b0, false)] {
        let host = HostFacts { ce: 0xc7b5, video };
        let cfg = kf_qemu::chan::translated_decoder_config(engine, &host, tier);
        assert_eq!(cfg.ce, 0);
        assert_eq!(cfg.video, video);
        assert_eq!(cfg.tier, tier);
        assert_eq!(kf_qemu::chan::ce_seed_log(cfg), "ce_seed=0x0");
    }
}

#[test]
fn every_family_selects_and_binds_each_host_advertised_ce_class() {
    use kf_arch::HostClasses;
    use kf_chip::classes::classes_for;
    for family in kf_chip::Family::ALL {
        let set = classes_for(family);
        for &ce in set.dma_copy {
            let advertised = [set.channel_gpfifo[0], set.usermode[0], ce];
            let derived =
                kf_chip::host_classes::DerivedHostClasses::choose(family, &advertised).unwrap();
            let class = derived.ce_object().ce_object_id().0;
            assert_eq!(class, ce, "{family:?}");
            let host = HostFacts {
                ce: class,
                video: 0,
            };
            let cfg = kf_qemu::chan::translated_decoder_config(0xb, &host, false);
            assert_eq!(cfg.ce, ce);
            let mut st = kf_chan::tmode::TState::default();
            st.gr = cfg;
            st.tier = kf_chan::ttables::Tier::of_ce_class(class);
            assert!(st.tier.is_some(), "{family:?} {class:#x}");
            let words = [kf_abi::submit::method_header_inc(4, 0, 1).unwrap(), ce];
            kf_chan::tmode::decode(&words, kf_chip::is_any_dma_copy_class, &mut st, None)
                .expect("the host-selected class binds on its own object");
        }
    }
}

fn advertised_classes_do_not_create_an_extra_object(family: kf_chip::Family) {
    use kf_arch::HostClasses;
    let set = kf_chip::classes::classes_for(family);
    assert_eq!(set.dma_copy.len(), 2);
    let advertised: Vec<_> = set
        .channel_gpfifo
        .iter()
        .chain(set.usermode)
        .chain(set.dma_copy)
        .copied()
        .collect();
    let chosen = kf_chip::host_classes::DerivedHostClasses::choose(family, &advertised).unwrap();
    let allocated = chosen.ce_object().ce_object_id().0;
    let older = *set.dma_copy.iter().min().unwrap();
    assert_ne!(allocated, older);
    let cfg = kf_qemu::chan::translated_decoder_config(
        0xb,
        &HostFacts {
            ce: allocated,
            video: 0,
        },
        false,
    );
    let mut st = kf_chan::tmode::TState::default();
    st.gr = cfg;
    st.tier = kf_chan::ttables::Tier::of_ce_class(allocated);
    let words = [kf_abi::submit::method_header_inc(4, 0, 1).unwrap(), older];
    assert!(matches!(
        kf_chan::tmode::decode(&words, kf_chip::is_any_dma_copy_class, &mut st, None),
        Err(kf_chan::translated::Refusal::ForeignClass { class, .. }) if class == older
    ));
}
#[test]
fn ampere_advertises_classes_but_the_ring_owns_one_selected_object() {
    advertised_classes_do_not_create_an_extra_object(kf_chip::Family::Ampere);
}
#[test]
fn blackwell_advertises_classes_but_the_ring_owns_one_selected_object() {
    advertised_classes_do_not_create_an_extra_object(kf_chip::Family::Blackwell);
}
