//! ★ The host side of GPU-utilisation reporting (`docs/design/V3_UTILIZATION_FLAG.md`, device property
//! `report-utilization`): the ONE fixed, host-authored, unprivileged read, over the host RM session.
//!
//! STATUS: LIVE, 2026-11-17 (GPU-free tests; **not run on hardware**).
//!
//! The request is [`kf_abi::utilization::host_request`]: a constant. It takes no argument, so nothing a guest
//! sent can reach it. The reply is parsed by [`kf_abi::utilization::decode_host_newest`] (offsets from the
//! generated, exact-per-tag layout) and reduced to six whole percentages; the rest of the 55 KiB reply is
//! dropped. The control is `NON_PRIVILEGED` (flags `0x50008`), the same call `nvidia-smi` makes as an ordinary
//! user on bare metal. It goes through [`kf_host::HostRm::raw_control`], so the host driver axis
//! (`kf_abi::hostabi::HOST_CONTROLS`) decides whether this host's layout is one we derived.
//!
//! The pure part ([`read_via`]) is tested against a fake control; [`HostUtil`] only binds it to the session.

use kf_abi::DriverVersion;
use kf_abi::utilization as ku;
use kf_host::RmError;
use kf_rm::utilization::{HostUtilization, UtilRefusal};

/// `NV_ERR_NOT_SUPPORTED`.
const NV_ERR_NOT_SUPPORTED: u32 = 0x56;

/// Classify a failed host call: a status that says "this host will never give it" is [`UtilRefusal::NoSource`]
/// (the device then does not serve the feature at all); anything else may pass and is [`UtilRefusal::Failed`].
#[must_use]
pub fn classify(e: &RmError) -> UtilRefusal {
    match e {
        // The unprivileged client is not allowed it, or the control is not supported on this GPU (MIG/SMC).
        RmError::InsufficientPermissions | RmError::Other(NV_ERR_NOT_SUPPORTED) => {
            UtilRefusal::NoSource(format!("{e:?}"))
        }
        // The host ABI axis refused to carry the control at this host driver (kf-host's named codes): the host
        // driver's layout was never derived, so no source here.
        RmError::Other(code)
            if *code == kf_host::ABI_ENCODE_FAILED || *code == kf_host::ABI_DECODE_FAILED =>
        {
            UtilRefusal::NoSource(format!("host ABI refused the control ({code:#x})"))
        }
        other => UtilRefusal::Failed(format!("{other:?}")),
    }
}

/// One host reading, with the control call abstracted: `host_version` is the host driver's version string and
/// `control(cmd, params)` issues the control on the host subdevice with `params` in and out.
///
/// # Errors
/// [`UtilRefusal`]: `NoSource` when the version does not parse or has no derived layout, or the host says it
/// will never answer; `Failed` for a transient failure; `Empty` when the host answered with no sample.
pub fn read_via(
    host_version: &str,
    control: &mut dyn FnMut(u32, &mut [u8]) -> Result<(), RmError>,
) -> Result<ku_reading::HostReading, UtilRefusal> {
    let version = DriverVersion::parse(host_version).ok_or_else(|| {
        UtilRefusal::NoSource(format!(
            "host driver version {host_version:?} does not parse"
        ))
    })?;
    let layout = ku::host_layout(version).ok_or_else(|| {
        UtilRefusal::NoSource(format!(
            "host driver {host_version}: no derived GPUMON layout (outside [580.65.06, 581) and not probed)"
        ))
    })?;
    // ★ The one request: a constant. No input of any kind.
    let mut params = ku::host_request(layout);
    control(u32::try_from(layout.gpumon_cmd).unwrap_or(0), &mut params)
        .map_err(|e| classify(&e))?;
    ku::decode_host_newest(layout, &params).ok_or(UtilRefusal::Empty)
}

/// Re-export so the signature above names one path.
pub mod ku_reading {
    pub use kf_abi::utilization::HostReading;
}

/// The production source: the host RM session's unprivileged subdevice.
pub struct HostUtil {
    rm: &'static kf_host::HostRm,
}

impl HostUtil {
    /// A source over `rm`.
    #[must_use]
    pub fn new(rm: &'static kf_host::HostRm) -> HostUtil {
        HostUtil { rm }
    }
}

impl HostUtilization for HostUtil {
    fn read(&mut self) -> Result<ku_reading::HostReading, UtilRefusal> {
        let rm = self.rm;
        read_via(rm.driver_version(), &mut |cmd, params| {
            rm.raw_control(rm.subdevice(), cmd, params)
        })
    }
}

/// ★ The property's decision, as one function: with `on == false` the host is never built, never read, nothing
/// is returned. `device.rs` calls exactly this with `cfg.report_utilization`.
#[must_use]
pub fn enable_if(
    on: bool,
    make_host: impl FnOnce() -> Box<dyn HostUtilization>,
) -> Option<UtilRuntime> {
    if !on {
        return None;
    }
    enable(make_host())
}

/// How often the sampler thread looks at its clock. The sampler itself decides whether a host call is due
/// ([`kf_rm::utilization::Sampler::tick`]: [`kf_rm::utilization::SAMPLE_PERIOD_MIN`] is the floor between calls).
const POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// ★ One VM's utilisation reporting, when the property is on and the host has a source: the shared state the
/// chain and the drainer use, and the sampler the `kf3-util` thread drives.
pub struct UtilRuntime {
    /// The shared state (the chain's seat holds a clone).
    pub state: std::sync::Arc<kf_rm::utilization::UtilizationState>,
    sampler: std::sync::Mutex<Option<kf_rm::utilization::Sampler>>,
}

/// Turn the property on for a host: build the state, take ONE reading to learn whether this host has an
/// unprivileged source at all, and return the runtime unless it says it never will. A transient failure or an
/// empty first reading keeps the feature on (the host may simply be idle); a permanent refusal logs once and
/// returns `None`, so the device behaves exactly as with the property off (design doc, section 3).
#[must_use]
pub fn enable(mut host: Box<dyn HostUtilization>) -> Option<UtilRuntime> {
    match host.read() {
        Err(UtilRefusal::NoSource(why)) => {
            eprintln!(
                "kf3: report-utilization=on, but this host has no unprivileged GPU-utilisation source ({why}): \
                 NOT serving utilisation; the guest sees N/A exactly as with report-utilization=off"
            );
            return None;
        }
        Err(other) => eprintln!(
            "kf3: report-utilization=on: first host reading was {other:?}; serving, will retry"
        ),
        Ok(_) => {}
    }
    eprintln!(
        "kf3: report-utilization=on: the guest is shown the PHYSICAL GPU's utilisation (whole device, 1 % resolution, \
         a cross-tenant load side channel on a shared GPU; set report-utilization=off to hide it)"
    );
    let state = kf_rm::utilization::UtilizationState::monotonic();
    let sampler = kf_rm::utilization::Sampler::new(host, state.clone());
    Some(UtilRuntime {
        state,
        sampler: std::sync::Mutex::new(Some(sampler)),
    })
}

impl UtilRuntime {
    /// The chain's seat.
    #[must_use]
    pub fn seat(&self) -> kf_rm::utilization::UtilizationSeat {
        kf_rm::utilization::UtilizationSeat {
            state: self.state.clone(),
        }
    }

    /// The `kf3-util` thread body: tick the sampler until `stop`. The host call inside `tick` is made with no
    /// lock held; nothing here is reachable from a vCPU. A panic in this thread only ends sampling (the snapshot
    /// then ages out and the guest sees N/A): it is deliberately not a fail-closed service thread.
    pub fn run(&self, stop: &std::sync::atomic::AtomicBool) {
        let Some(mut sampler) = self
            .sampler
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        while !stop.load(std::sync::atomic::Ordering::Acquire) {
            let _ = sampler.tick();
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(l: &ku::Layout, ts: u64, util: u32) -> Vec<u8> {
        let mut p = ku::host_request(l);
        let at = usize::try_from(l.off_samples).unwrap();
        p[at..at + 8].copy_from_slice(&ts.to_le_bytes());
        let gr = at + usize::try_from(l.off_gr + l.eng_off_util).unwrap();
        p[gr..gr + 4].copy_from_slice(&util.to_le_bytes());
        p
    }

    #[test]
    fn the_request_is_the_constant_one_and_nothing_else_is_sent() {
        let l = ku::host_layout(DriverVersion {
            major: 580,
            minor: 159,
            patch: 4,
        })
        .unwrap();
        let mut seen: Vec<(u32, Vec<u8>)> = Vec::new();
        let r = read_via("580.159.04", &mut |cmd, p| {
            seen.push((cmd, p.to_vec()));
            p.copy_from_slice(&reply(l, 9, 4_200));
            Ok(())
        })
        .expect("a reading");
        assert_eq!(seen.len(), 1, "exactly one host call");
        assert_eq!(seen[0].0, 0x2080_2096);
        assert_eq!(seen[0].1, ku::host_request(l), "the constant request");
        assert_eq!(seen[0].1.len(), 55_888);
        assert_eq!(r.percents.gpu, 42);
        assert_eq!(r.host_timestamp, 9);
        // calling again sends the same bytes (no state, no input)
        let mut again = Vec::new();
        let _ = read_via("580.159.04", &mut |_, p| {
            again = p.to_vec();
            Err(RmError::Other(0x1f))
        });
        assert_eq!(again, seen[0].1);
    }

    #[test]
    fn an_unparseable_or_underived_host_is_no_source_and_no_call_is_made() {
        for v in ["", "garbage", "580", "535.309.01", "581.0", "595.91.8"] {
            let r = read_via(v, &mut |_, _| panic!("no host call for {v:?}"));
            assert!(matches!(r, Err(UtilRefusal::NoSource(_))), "{v:?}: {r:?}");
        }
    }

    #[test]
    fn a_host_that_will_never_answer_is_no_source_and_a_transient_failure_is_not() {
        let call = |e: RmError| read_via("580.159.04", &mut |_, _| Err(e.clone()));
        for e in [
            RmError::InsufficientPermissions,
            RmError::Other(0x56),
            RmError::Other(kf_host::ABI_ENCODE_FAILED),
            RmError::Other(kf_host::ABI_DECODE_FAILED),
        ] {
            assert!(
                matches!(call(e.clone()), Err(UtilRefusal::NoSource(_))),
                "{e:?}"
            );
        }
        for e in [
            RmError::Other(0x1f),
            RmError::NoMemory,
            RmError::Interrupted,
        ] {
            assert!(
                matches!(call(e.clone()), Err(UtilRefusal::Failed(_))),
                "{e:?}"
            );
        }
    }

    struct Fixed(Result<ku_reading::HostReading, UtilRefusal>);
    impl HostUtilization for Fixed {
        fn read(&mut self) -> Result<ku_reading::HostReading, UtilRefusal> {
            self.0.clone()
        }
    }

    fn some_reading() -> ku_reading::HostReading {
        ku_reading::HostReading {
            host_timestamp: 1,
            percents: Default::default(),
            clamped: 0,
        }
    }

    #[test]
    fn enable_serves_unless_the_host_says_it_never_will() {
        assert!(enable(Box::new(Fixed(Ok(some_reading())))).is_some());
        assert!(
            enable(Box::new(Fixed(Err(UtilRefusal::Empty)))).is_some(),
            "an idle host"
        );
        assert!(
            enable(Box::new(Fixed(Err(UtilRefusal::Failed("x".into()))))).is_some(),
            "transient"
        );
        assert!(
            enable(Box::new(Fixed(Err(UtilRefusal::NoSource("MIG".into()))))).is_none(),
            "a host with no source: the device behaves as with the property off"
        );
    }

    #[test]
    fn the_property_off_builds_no_host_makes_no_read_and_returns_nothing() {
        let made = std::cell::Cell::new(false);
        let r = enable_if(false, || {
            made.set(true);
            Box::new(Fixed(Ok(some_reading())))
        });
        assert!(r.is_none());
        assert!(!made.get(), "the host source is not even constructed");
        assert!(enable_if(true, || Box::new(Fixed(Ok(some_reading())))).is_some());
    }

    #[test]
    fn a_host_that_refuses_with_not_supported_turns_the_feature_off_through_the_real_read_path() {
        // read_via over a control that answers NV_ERR_NOT_SUPPORTED (a MIG host), composed with enable().
        struct ViaNotSupported;
        impl HostUtilization for ViaNotSupported {
            fn read(&mut self) -> Result<ku_reading::HostReading, UtilRefusal> {
                read_via("580.159.04", &mut |_, _| Err(RmError::Other(0x56)))
            }
        }
        assert!(enable(Box::new(ViaNotSupported)).is_none());
    }

    #[test]
    fn the_request_is_the_same_constant_whatever_the_host_driver() {
        for v in [
            "580.65.06",
            "580.159.04",
            "580.142.03",
            "595.91.07",
            "615.71.09",
        ] {
            let l = ku::host_layout(DriverVersion::parse(v).unwrap()).unwrap();
            let mut sent = Vec::new();
            let _ = read_via(v, &mut |_, p| {
                sent = p.to_vec();
                Ok(())
            });
            assert_eq!(sent, ku::host_request(l), "{v}");
            assert_eq!(sent.len(), 55_888, "{v}");
        }
    }

    #[test]
    fn the_sampler_thread_samples_while_asked_and_stops_on_the_flag() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Counting(u64, std::sync::Arc<std::sync::atomic::AtomicU64>);
        impl HostUtilization for Counting {
            fn read(&mut self) -> Result<ku_reading::HostReading, UtilRefusal> {
                self.0 += 1;
                self.1.fetch_add(1, Ordering::Relaxed);
                Ok(ku_reading::HostReading {
                    host_timestamp: self.0,
                    percents: Default::default(),
                    clamped: 0,
                })
            }
        }
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let rt = std::sync::Arc::new(enable(Box::new(Counting(0, calls.clone()))).unwrap());
        rt.state.note_interest();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let t = {
            let (rt, stop) = (rt.clone(), stop.clone());
            std::thread::spawn(move || rt.run(&stop))
        };
        std::thread::sleep(std::time::Duration::from_millis(700));
        stop.store(true, Ordering::Release);
        t.join().unwrap();
        // the probe read (1) + the first tick + about two more at 250 ms while asked; never more than the floor allows
        let n = calls.load(Ordering::Relaxed);
        assert!((2..=8).contains(&n), "{n} host calls in 0.7 s");
        assert!(rt.state.fresh_samples().is_some(), "a sample was stored");
        // a second run() finds the sampler already taken and returns at once
        rt.run(&AtomicBool::new(false));
    }

    #[test]
    fn a_reply_with_no_sample_is_empty_not_zero() {
        let r = read_via("580.159.04", &mut |_, _| Ok(()));
        assert_eq!(r, Err(UtilRefusal::Empty));
    }
}
