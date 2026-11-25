//! ★ 2026-11-15 (Flag cleanup, Stage 1; owner: "set the measured Windows profile as the default"):
//! the **Windows support profile** — one validated set of Windows-only behaviours that is ON by
//! default and keyed on what the guest DECLARES, never on an environment variable.
//!
//! Two inputs decide whether a profile behaviour may be reached, and both are required:
//!
//! 1. **`windows-support`** (the device property, default on; [`WindowsProfile::support`]): the one
//!    coarse opt-out. `off` is a Linux-only VM: no Windows behaviour can be reached, even by a guest
//!    that declares a Windows identity (`docs/design/V3_WINDOWS_SUPPORT_PROFILE.md`).
//! 2. **The declared identity** (fn 1, `SET_GUEST_SYSTEM_INFO`; [`declares_windows`]): the guest said
//!    it is the measured Windows build. Every new declaration replaces the previous one. It is NOT a
//!    security boundary (a guest can lie; owner rule D4): a lie changes only that guest's own
//!    compatibility behaviour, and every behaviour here is authored by the host from unprivileged
//!    verbs, never forwarded from guest bytes. A Linux guest never declares it, so a Linux guest
//!    reaches none of the behaviours — by construction, not by measurement.
//!
//! ⊘ What the profile adds, named as what it is: these became
//! PRODUCT behaviour for a declared Windows guest, and several are **fabricated answers**
//! (class (c): a value the guest accepts that no host query produced) — the `GFX_POOL_QUERY_SIZE`
//! sizes ([`crate::gfxpool_probe`]), the `GR_CTXSW_PREEMPTION_BIND` / `GR_CTXSW_ZCULL_BIND` `NV_OK`
//! answers with nothing bound on the host ([`crate::preempt_bind_probe`]), the six echoed NV0073
//! controls ([`crate::display_ctrl_probe`]) and the software-runlist `NV_OK` with its contents
//! ignored ([`crate::sw_runlist_host`]). They were measured as needed (Windows Code 43 without them)
//! and they claim no GPU work.

use kf_gsp::{CommandObserver, CommandPolicy, Deferred, Reply, RpcCommand, RpcFunction};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Whether `payload` (a `SET_GUEST_SYSTEM_INFO` body) declares the measured Windows build of the
/// guest driver `version`. Pure; an unmeasured version, a malformed payload or a Linux declaration
/// is `false`. The identity selects a profile; it is not a security boundary (module doc).
#[must_use]
pub fn declares_windows(version: kf_abi::DriverVersion, payload: &[u8]) -> bool {
    kf_abi::hoststub::cell(version).is_some_and(|c| c.matches_identity(payload))
}

/// The profile's state, shared by every link, plane and thread that consults it.
///
/// `Clone` hands out the SAME declaration cell (`Arc` inside): the device makes one per VM, the
/// chain's identity observer writes it at every fn 1, and the display and memory planes read it.
#[derive(Debug, Clone)]
pub struct WindowsProfile {
    support: bool,
    declared: Arc<AtomicBool>,
}

impl Default for WindowsProfile {
    /// `windows-support=on` (the default), nothing declared yet.
    fn default() -> WindowsProfile {
        WindowsProfile::new(true)
    }
}

impl WindowsProfile {
    /// A profile with the property set to `support`.
    #[must_use]
    pub fn new(support: bool) -> WindowsProfile {
        WindowsProfile {
            support,
            declared: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `windows-support=on`.
    #[must_use]
    pub fn on() -> WindowsProfile {
        WindowsProfile::new(true)
    }

    /// `windows-support=off`: the Windows behaviours cannot be reached by any declaration.
    #[must_use]
    pub fn off() -> WindowsProfile {
        WindowsProfile::new(false)
    }

    /// The `windows-support` property.
    #[must_use]
    pub fn support(&self) -> bool {
        self.support
    }

    /// May a Windows behaviour be reached NOW: the property is on AND the guest's current
    /// declaration is the Windows 580.88 identity. The one question every gate asks.
    #[must_use]
    pub fn active(&self) -> bool {
        self.support && self.declared.load(Ordering::Acquire)
    }

    /// Record the guest's current declaration (every fn 1 replaces the last). With the property
    /// off nothing is ever recorded, so [`WindowsProfile::active`] stays `false`.
    pub fn declare(&self, windows: bool) {
        self.declared
            .store(self.support && windows, Ordering::Release);
    }
}

/// The chain's front observer: reads every fn 1 and records whether it declares Windows. It cannot
/// answer ([`CommandObserver`] has no return value), so seating it first changes no reply.
pub struct IdentityObserver {
    version: kf_abi::DriverVersion,
    profile: WindowsProfile,
}

impl IdentityObserver {
    /// An observer for the guest driver `version`, writing `profile`.
    #[must_use]
    pub fn new(version: kf_abi::DriverVersion, profile: WindowsProfile) -> IdentityObserver {
        IdentityObserver { version, profile }
    }
}

impl CommandObserver for IdentityObserver {
    fn observe(&mut self, cmd: &RpcCommand) {
        if cmd.function == RpcFunction::SetGuestSystemInfo {
            self.profile
                .declare(declares_windows(self.version, &cmd.payload));
        }
    }
}

/// The line a Windows guest without the display plane is refused with (the QEMU log; one per rate
/// window).
pub const WINDOWS_NEEDS_DISPLAY: &str = "kf3: WINDOWS GUEST REFUSED: the guest declared the Windows driver and this device has no display plane (display=off). NVIDIA's Windows driver resets itself without one. Start the VM with display=on,gop=on on the kf3 device and -vga none (docs/howto/windows-guest.md), or set windows-support=off for a Linux-only VM";

/// The link that refuses a Windows guest on a device without the display plane, by name, at its
/// fn 1: the guest driver's first call fails with `NV_ERR_NOT_SUPPORTED` and the device does not
/// start in the guest, instead of the driver starting and then resetting itself.
///
/// It is seated only when `windows-support` is on and the device was realized with `display=off`
/// ([`WindowsNeedsDisplay::seat`]); it answers fn 1 only when the payload declares the
/// Windows build of the guest driver ([`declares_windows`]) and nothing else, so a Linux guest's fn 1 falls through to
/// the next link unchanged. The declaration is not a security boundary: a guest that declares
/// Windows falsely is refused its own driver start and nothing else.
pub struct WindowsNeedsDisplay {
    version: kf_abi::DriverVersion,
}

impl WindowsNeedsDisplay {
    /// The link for a device with `windows_support` and `display_plane` as given: `Some` exactly
    /// when the profile is on and there is no display plane.
    #[must_use]
    pub fn seat(
        version: kf_abi::DriverVersion,
        windows_support: bool,
        display_plane: bool,
    ) -> Option<WindowsNeedsDisplay> {
        (windows_support && !display_plane).then_some(WindowsNeedsDisplay { version })
    }
}

impl CommandPolicy for WindowsNeedsDisplay {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::SetGuestSystemInfo
            || !declares_windows(self.version, &cmd.payload)
        {
            return None;
        }
        kf_arch::rate_eprintln!("{WINDOWS_NEEDS_DISPLAY}");
        Some(Reply {
            rpc_result_private: None,
            rpc_result: kf_abi::NV_ERR_NOT_SUPPORTED,
            body: Vec::new(),
        })
    }
}

/// A link that is reachable only while [`WindowsProfile::active`]: with a Linux declaration, no
/// declaration, or `windows-support=off`, it answers nothing, holds nothing and defers nothing, so
/// the command falls through to the next link exactly as if the link were not in the chain.
///
/// ⊘ It does not decide the identity itself: [`IdentityObserver`] (seated ahead of every wrapped
/// link) does, once per fn 1, so all wrapped links agree.
pub struct WindowsOnly<P: CommandPolicy> {
    inner: P,
    profile: WindowsProfile,
}

impl<P: CommandPolicy> WindowsOnly<P> {
    /// Gate `inner` on `profile`.
    #[must_use]
    pub fn new(inner: P, profile: WindowsProfile) -> WindowsOnly<P> {
        WindowsOnly { inner, profile }
    }
}

impl<P: CommandPolicy> CommandPolicy for WindowsOnly<P> {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if self.profile.active() {
            self.inner.respond(cmd)
        } else {
            None
        }
    }

    fn holds_for_refresh(&self, cmd: &RpcCommand) -> bool {
        self.profile.active() && self.inner.holds_for_refresh(cmd)
    }

    fn defers(&mut self, cmd: &RpcCommand) -> Option<Deferred> {
        if self.profile.active() {
            self.inner.defers(cmd)
        } else {
            None
        }
    }
}

impl<P: CommandPolicy> core::fmt::Debug for WindowsOnly<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WindowsOnly")
            .field("active", &self.profile.active())
            .finish_non_exhaustive()
    }
}

/// The fn-1 fixtures the integration tests share (`tests/support/windows_fn1.rs`).
#[cfg(test)]
#[path = "../tests/support/windows_fn1.rs"]
pub(crate) mod fn1;

#[cfg(test)]
mod tests {
    use super::fn1;
    use super::*;

    /// A policy that answers every command, so a gate is the only thing standing between a command
    /// and an answer.
    struct AnswersEverything;
    impl CommandPolicy for AnswersEverything {
        fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
            Some(Reply {
                rpc_result_private: None,
                rpc_result: 0,
                body: cmd.payload.clone(),
            })
        }
        fn holds_for_refresh(&self, _: &RpcCommand) -> bool {
            true
        }
    }

    fn any_control() -> RpcCommand {
        RpcCommand {
            function: RpcFunction::RmControl,
            code: 0,
            sequence: 1,
            payload: vec![1, 2, 3],
            elements: 1,
            delivered: Vec::new(),
        }
    }

    #[test]
    fn the_property_off_is_never_active_whatever_is_declared() {
        let p = WindowsProfile::off();
        p.declare(true);
        assert!(!p.active());
        assert!(!p.support());
    }

    #[test]
    fn the_property_on_is_active_only_after_a_windows_declaration() {
        let p = WindowsProfile::default();
        assert!(p.support());
        assert!(!p.active(), "nothing declared yet");
        p.declare(true);
        assert!(p.active());
        p.declare(false);
        assert!(!p.active(), "a later (Linux) declaration revokes");
    }

    #[test]
    fn a_clone_shares_the_declaration() {
        let a = WindowsProfile::on();
        let b = a.clone();
        a.declare(true);
        assert!(b.active());
    }

    const WRAPPER_ARMS: [(bool, bool, bool); 4] = [
        // (property on, declared Windows, reachable)
        (true, false, false),
        (true, true, true),
        (false, true, false),
        (false, false, false),
    ];

    fn wrapper(on: bool, declared: bool) -> WindowsOnly<AnswersEverything> {
        let profile = WindowsProfile::new(on);
        profile.declare(declared);
        WindowsOnly::new(AnswersEverything, profile)
    }

    #[test]
    fn the_wrapper_answers_nothing_unless_active() {
        let c = any_control();
        for (on, declared, reachable) in WRAPPER_ARMS {
            assert_eq!(
                wrapper(on, declared).respond(&c).is_some(),
                reachable,
                "on={on} declared={declared}"
            );
        }
    }

    #[test]
    fn the_wrapper_holds_nothing_unless_active() {
        let c = any_control();
        for (on, declared, reachable) in WRAPPER_ARMS {
            assert_eq!(
                wrapper(on, declared).holds_for_refresh(&c),
                reachable,
                "on={on} declared={declared}"
            );
        }
    }

    /// A chain holds a reply for a refresh only while the wrapper holding it is active: the gate is
    /// inside the wrapper, so it also holds through [`kf_gsp::PolicyChain`], which asks every link.
    #[test]
    fn a_chain_of_inactive_wrappers_holds_no_reply() {
        let c = any_control();
        for (on, declared, reachable) in WRAPPER_ARMS {
            let profile = WindowsProfile::new(on);
            profile.declare(declared);
            let chain = kf_gsp::PolicyChain::new(vec![Box::new(WindowsOnly::new(
                AnswersEverything,
                profile,
            ))]);
            assert_eq!(
                chain.holds_for_refresh(&c),
                reachable,
                "on={on} declared={declared}"
            );
        }
    }

    /// The identity is read from the guest's fn 1 and nothing else: Windows declares it, Linux does
    /// not, a later Linux declaration revokes it, and `windows-support=off` never records it.
    #[test]
    fn the_observer_records_exactly_what_fn1_declares() {
        let abi = fn1::abi();
        let v = abi.driver_version();
        let (win, lin) = (fn1::windows(&abi), fn1::linux(&abi));
        assert!(
            declares_windows(v, &win.payload),
            "the measured Windows build"
        );
        assert!(
            !declares_windows(v, &lin.payload),
            "the Linux build of the same wire"
        );

        let on = WindowsProfile::on();
        let mut obs = IdentityObserver::new(v, on.clone());
        obs.observe(&lin);
        assert!(!on.active(), "a Linux guest never activates the profile");
        obs.observe(&win);
        assert!(on.active());
        obs.observe(&any_control());
        assert!(on.active(), "only fn 1 declares");
        obs.observe(&lin);
        assert!(!on.active(), "every declaration replaces the last");

        let off = WindowsProfile::off();
        let mut obs = IdentityObserver::new(v, off.clone());
        obs.observe(&win);
        assert!(
            !off.active(),
            "windows-support=off: a Windows declaration reaches nothing"
        );
    }

    /// A Windows guest's fn 1 on a device without the display plane is refused with
    /// `NV_ERR_NOT_SUPPORTED` and an empty body; a Linux guest's fn 1 and every other command fall
    /// through.
    #[test]
    fn a_windows_fn1_without_the_display_plane_is_refused_by_name() {
        let abi = fn1::abi();
        let v = abi.driver_version();
        let (win, lin) = (fn1::windows(&abi), fn1::linux(&abi));
        let mut link = WindowsNeedsDisplay::seat(v, true, false).expect("seated");
        let reply = link.respond(&win).expect("a Windows fn 1 is answered");
        assert_eq!(reply.rpc_result, kf_abi::NV_ERR_NOT_SUPPORTED);
        assert_eq!(reply.rpc_result_private, None);
        assert!(reply.body.is_empty());
        assert!(link.respond(&lin).is_none(), "a Linux fn 1 falls through");
        assert!(link.respond(&any_control()).is_none(), "only fn 1");
        assert!(!link.holds_for_refresh(&win));
        assert!(link.defers(&win).is_none());
        assert!(WINDOWS_NEEDS_DISPLAY.contains("display=on,gop=on"));
        assert!(WINDOWS_NEEDS_DISPLAY.contains("docs/howto/windows-guest.md"));
    }

    /// The link exists only with `windows-support` on and no display plane: a device with the
    /// display plane, and a Linux-only VM, have no such link.
    #[test]
    fn the_refusal_is_seated_only_without_a_display_plane_and_with_windows_support() {
        let v = fn1::abi().driver_version();
        for (support, display, seated) in [
            (true, false, true),
            (true, true, false),
            (false, false, false),
            (false, true, false),
        ] {
            assert_eq!(
                WindowsNeedsDisplay::seat(v, support, display).is_some(),
                seated,
                "windows_support={support} display_plane={display}"
            );
        }
    }

    /// Ahead of a link that would accept the fn 1, the refusal wins for a Windows guest and the
    /// accepting link still answers a Linux guest.
    #[test]
    fn in_a_chain_the_refusal_answers_before_the_fn1_link() {
        let abi = fn1::abi();
        let v = abi.driver_version();
        let (win, lin) = (fn1::windows(&abi), fn1::linux(&abi));
        let mut chain = kf_gsp::PolicyChain::new(vec![
            Box::new(WindowsNeedsDisplay::seat(v, true, false).expect("seated")),
            Box::new(AnswersEverything),
        ]);
        assert_eq!(
            chain.respond(&win).map(|r| r.rpc_result),
            Some(kf_abi::NV_ERR_NOT_SUPPORTED)
        );
        assert_ne!(
            chain.respond(&lin).map(|r| r.rpc_result),
            Some(kf_abi::NV_ERR_NOT_SUPPORTED),
            "a Linux guest is not refused"
        );
    }

    #[test]
    fn an_unmeasured_driver_or_a_malformed_payload_declares_nothing() {
        let v = kf_abi::DriverVersion {
            major: 1,
            minor: 2,
            patch: 3,
        };
        assert!(!declares_windows(v, &[]));
        assert!(!declares_windows(v, &[0xff; 4096]));
        let measured = fn1::abi().driver_version();
        assert!(!declares_windows(measured, &[]));
        assert!(!declares_windows(measured, &[0u8; 4096]));
    }
}
