//! # The fourth axis — the **guest operating system**, given a home
//!
//! `four_axes_of_variation.md` §1 names four axes of variation and says of this one:
//! *"the fourth axis is the one to protect now, because it is the only one with no
//! home — `DriverVersion` silently means 'Linux, ogkm-shaped'"*. This module is that
//! home: a [`GuestOs`] profile that sits **beside** [`crate::versions::DriverAbiTable`]
//! rather than inside it, because the doc's next paragraph is *"do not collapse guest OS
//! into the version key"* — one key spanning two independent dimensions is the C's
//! major-only version-key mistake one level up.
//!
//! # 1. ★★★ The defect this closes, and why its SHAPE is the bad part
//!
//! Until 2026-07-29 the classification below was a free function:
//!
//! ```text
//! fn client_kind_from_process_id(process_id: u32) -> ClientKind {
//!     if process_id == KERNEL_PID { Kernel } else { User { pid: process_id } }
//! }
//! ```
//!
//! `KERNEL_PID` is a sentinel RM stamps on a kernel-privileged client's
//! `NV0000_ALLOC_PARAMETERS.processID` — **and it is gated on a driver build flag**:
//!
//! ```text
//! if (RMCFG_FEATURE_PLATFORM_UNIX &&
//!    (pCallContext->secInfo.privLevel >= RS_PRIV_LEVEL_KERNEL))
//!     root_alloc_params.processID = KERNEL_PID;
//! else
//!     root_alloc_params.processID = pClient->ProcID;
//! ```
//!
//! (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` /
//! `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` — byte-identical, same lines at
//! both tags.)
//!
//! On a **non-UNIX** guest the `else` arm runs for a kernel-privileged client too, so the
//! WDDM kernel's RM clients declare a *real* pid — and the old function classified every
//! one of them as `ClientKind::User { pid }`. Decision #14 groups a host isolate per
//! `ClientKind::User` pid, so the guest kernel's clients would be **folded into a guest
//! process's blast radius**, and a second guest process declaring the same pid value
//! would join it.
//!
//! ★★ The severity is in the shape, not the size. It was not a refusal, not a wrong
//! answer that stops anything, and not a value anybody logs — it was a *plausible*
//! answer, produced silently, on the one path whose whole job is to decide who may
//! reach whose memory. Replacing it with a different silent answer would not be a fix,
//! which is why every arm below that does not KNOW terminates in a typed refusal.
//!
//! # 2. ★ Configuration, never detection
//!
//! **The guest OS is undetectable on the wire.** It is a `#define` in the guest driver's
//! build, not a protocol field: the RPC envelope, the msgq header and every alloc/control
//! payload are identical between a Linux and a Windows guest running the same driver
//! branch. There is therefore no observation that distinguishes them, and any function
//! that claimed to would be inferring a security boundary from a coincidence.
//!
//! So the profile is **declared** by whoever realizes the device, exactly as
//! [`crate::DriverVersion`] is. A heuristic here — "the first client declared
//! `0xFFFFFFFF`, so it must be Linux" — would be the same silent fold with a plausible
//! story attached, and it would be *attacker-controlled*: the first `NV01_ROOT` on the
//! wire is a value the guest picks.
//!
//! This also settles what "unknown" means. An OS we cannot name is refused at
//! configuration time ([`UnknownGuestOsName`]); an OS we can name but whose rule we have
//! not established is refused at classification time ([`ClientKindRuleUnknown`]). Neither
//! is a fallback to Linux.
//!
//! # 3. Why Windows is a NAMED REFUSAL and not an implementation
//!
//! We know the Linux rule *because it was measured* (RTX 3060 / 580.159.04: two
//! concurrent CUDA processes declared their own pids, the UVM session client and every
//! other RM-internal client declared the sentinel). We know **nothing equivalent** about
//! WDDM. The `else` arm above tells us the sentinel is not used; it does not tell us what
//! *is*, whether kernel clients are distinguishable at all on that path, or whether the
//! `pOsPidInfo` field it also populates is the discriminator.
//!
//! A confident wrong rule here is strictly worse than a refusal: a refusal costs a
//! Windows guest a boot it cannot currently have anyway, while a wrong rule silently
//! re-creates the exact fold this module exists to close. So [`GuestOs::Windows`] is
//! present — named, so the seam is a grep away — and its rule is [`None`].
//!
//! ★ **Adding it later is one line.** The rule is data ([`ClientKindRule`]), so a
//! measured Windows rule is a new [`ClientKindRule`] variant plus moving `Windows` from
//! the `None` arm of [`GuestOs::client_kind_rule`] to a `Some`. Nothing above the ABI
//! seam changes; no trait changes at all.
//!
//! # 4. ★★ A SECOND gate on the same field, found while writing this
//!
//! The `RMCFG_FEATURE_PLATFORM_UNIX` test is not the only condition on `processID`. The
//! whole block sits under `if (!IsT234DorBetter(pGpu))` (`ogkm-580: rpc.h:57` /
//! `ogkm-610: rpc.h:57`), and the struct is zero-initialised at `rpc.h:53`
//! (`NV0000_ALLOC_PARAMETERS root_alloc_params = {0};`). So on a **T234D-or-better**
//! (Orin-class) GPU, *no* `processID` is ever written and **every** client — kernel and
//! user alike — declares `0`.
//!
//! Under the Linux rule that is `User { pid: 0 }` for all of them: not a fold of the
//! kernel into one process, but a collapse of the **entire guest** into a single
//! `ClientKind`. That is a *chip*-axis gate hiding inside what the seam audit costed as
//! an OS-axis defect, and it means this one field is conditioned on two independent axes
//! rather than one.
//!
//! It is recorded rather than fixed, deliberately, and the boundary is exact:
//! - our target is GA106 (`mode2_plan.md`), which is not T234D-or-better, so the Linux
//!   rule is sound **for the hardware we run** and this changes no behaviour today;
//! - inventing a `process_id == 0` refusal would change Linux behaviour on the strength
//!   of a code path we have never observed, which is the same "confident rule from a
//!   reading" this module refuses for Windows;
//! - a Tegra guest is not merely an untested configuration for this function, it is a
//!   configuration where the function is *known* to be wrong. When one becomes a target,
//!   the answer is another [`ClientKindRule`] here — the same one-line shape as Windows —
//!   and `guest_os_axis_gate.rs`'s Tegra characterisation test is where that is asserted.

use kayfabe_arch::ClientKind;

/// `KERNEL_PID` — the reserved `processID` RM stamps on a **kernel-privileged** client's
/// `NV01_ROOT` alloc params, on a **UNIX** guest only
/// (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` /
/// `ogkm-610: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` — byte-identical, same lines at
/// both: `RMCFG_FEATURE_PLATFORM_UNIX && privLevel >= RS_PRIV_LEVEL_KERNEL → processID =
/// KERNEL_PID`, else the client's own `ProcID`).
///
/// This is an NVIDIA wire constant, so per the quarantine rule (decision #2) it exists
/// **only in this crate**; the logic crates speak [`ClientKind`]. It is additionally
/// contained to **this module** — the one home of the OS-conditional rule — and
/// `tests/tests/guest_os_axis_gate.rs` asserts that containment, because a second copy
/// of this sentinel is a second place the OS gate can be forgotten.
const KERNEL_PID: u32 = 0xFFFF_FFFF;

/// Which guest operating system's driver is on the other end of the RPC plane.
///
/// **Declared at realize, never detected** — see the module docs §2. One value per OS,
/// and the OS-conditional rules hang off it as data ([`ClientKindRule`]) rather than as
/// branches, so a new OS is a table row and not a code path.
///
/// ★ Deliberately **not** `#[non_exhaustive]`. A downstream `match` on this enum should
/// break when a new OS is added: that break is the review prompt asking whether the
/// site's assumption still holds. `#[non_exhaustive]` would force a `_` arm at every
/// such site, which is a silent default — the precise failure mode this type exists to
/// remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum GuestOs {
    /// A Linux guest running the ogkm-shaped driver — the only guest we can currently
    /// boot, and the only one whose rules are **measured** rather than read: RTX 3060 /
    /// 580.159.04, the run written up in this module's own docs and in
    /// `l1_concurrency.md` §12.27. The measurement is named here rather than left as a
    /// bare adjective, because "measured rather than read" is the exact assertion that
    /// must not be inherited on trust.
    ///
    /// ★ This is [`Default`], and that default is a *declaration*, not a detection: it
    /// is the value the (not-yet-built) realize-time configuration starts from, chosen
    /// because it is the only guest the stack can run. It is reachable only from
    /// configuration — every classification site names its `GuestOs` explicitly, so no
    /// code path falls back to this value by omission.
    #[default]
    Linux,
    /// A Windows guest running the WDDM driver.
    ///
    /// ★★ Named so that the seam is a grep away (`four_axes_of_variation.md` §1: *"that
    /// only stays true if nothing bakes in 'the guest is Linux'"*), and **unimplemented
    /// on purpose**. Every rule below answers [`None`] for it, which is a typed refusal
    /// at the call site rather than a guess. See the module docs §3.
    Windows,
}

/// The names [`GuestOs::from_config_name`] accepts, in one place so an operator-facing
/// error can list them.
pub const GUEST_OS_CONFIG_NAMES: &[&str] = &["linux", "windows"];

/// An operator asked for a guest OS this build has no profile for.
///
/// ★ The **configuration-time** half of "unknown must refuse" (module docs §2). A typo in
/// `--guest-os=` must not become [`GuestOs::Linux`]: silently serving a Windows guest the
/// Linux profile is exactly the fold this module closes, and a mistyped flag is the
/// cheapest way to reach it. The accepted spellings are [`GUEST_OS_CONFIG_NAMES`]; the
/// caller still holds the string it passed, so this carries no copy of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownGuestOsName;

/// How a guest's RM declares that a client is **kernel**-privileged rather than a user
/// process's.
///
/// The rule is **data**, so that "which OS do we know this for?" is a table
/// ([`GuestOs::client_kind_rule`]) rather than a chain of `if`s spread across the
/// bridge — and so that a refusal can *name* the rule it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKindRule {
    /// `processID == KERNEL_PID` (`0xFFFF_FFFF`) **iff** kernel-privileged; any other
    /// value is that client's own guest pid.
    ///
    /// Measured, not inferred (RTX 3060 / 580.159.04, `l1_concurrency.md` §12.27): the
    /// two concurrent CUDA processes' clients declared their own pids, while the single
    /// UVM session client and every other RM-internal client declared the sentinel. The
    /// driver source it matches is `ogkm-580: rpc.h:67-77` / `ogkm-610: rpc.h:67-77`.
    ///
    /// ★ Sound for our target hardware only — see the module docs §4 for the
    /// `IsT234DorBetter` gate that makes this rule wrong on Orin-class parts.
    KernelPidSentinel,
}

/// The guest OS is named, but the rule for *this* question has not been established for
/// it.
///
/// ★★ The **classification-time** half of "unknown must refuse". Carries both the profile
/// that was configured and the value that could not be classified, so a census entry
/// says which guest hit it and a trace says what it was holding — a refusal that only
/// said "no" would leave the reader unable to tell a misconfiguration from a driver
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientKindRuleUnknown {
    /// The configured profile whose [`ClientKindRule`] is [`None`].
    pub guest_os: GuestOs,
    /// The `NV0000_ALLOC_PARAMETERS.processID` that arrived, unclassified and
    /// uninterpreted.
    pub process_id: u32,
}

impl GuestOs {
    /// Parse an operator-supplied profile name, ASCII-case-insensitively.
    ///
    /// ★ The **one door** into this type from configuration, and it refuses rather than
    /// defaults. The realize-time plumbing that will call it is not built yet
    /// (`four_axes_of_variation.md` §1: the axis had no home at all until now), so this
    /// is deliberately the smallest thing that can be the door: no aliases, no prefix
    /// matching, no "did you mean". Every one of those is a way for a wrong name to
    /// become a plausible profile.
    ///
    /// # Errors
    ///
    /// [`UnknownGuestOsName`] for anything not in [`GUEST_OS_CONFIG_NAMES`].
    pub fn from_config_name(name: &str) -> Result<GuestOs, UnknownGuestOsName> {
        if name.eq_ignore_ascii_case("linux") {
            Ok(GuestOs::Linux)
        } else if name.eq_ignore_ascii_case("windows") {
            Ok(GuestOs::Windows)
        } else {
            Err(UnknownGuestOsName)
        }
    }

    /// The name this profile is configured by — the inverse of
    /// [`Self::from_config_name`], for diagnostics.
    #[must_use]
    pub const fn config_name(self) -> &'static str {
        match self {
            GuestOs::Linux => "linux",
            GuestOs::Windows => "windows",
        }
    }

    /// How this guest's RM declares kernel privilege — or [`None`] if we do not know.
    ///
    /// ★★★ **The whole fourth-axis seam is this one `match`.** Every arm is written out;
    /// there is no `_`, so adding a [`GuestOs`] variant is a compile error here, which is
    /// the point. `None` is not "assume the usual" — every caller must turn it into a
    /// refusal.
    #[must_use]
    pub const fn client_kind_rule(self) -> Option<ClientKindRule> {
        match self {
            GuestOs::Linux => Some(ClientKindRule::KernelPidSentinel),
            // ★ NOT a guess. See the module docs §3: the driver source tells us the
            // sentinel is *not* used on a non-UNIX build, and tells us nothing about what
            // replaces it. One line moves this to `Some(..)` the day it is measured.
            GuestOs::Windows => None,
        }
    }

    /// Decode a declared `processID` from `NV0000_ALLOC_PARAMETERS` into the abstract
    /// [`ClientKind`] the core groups on (`l1_concurrency.md` §12.27).
    ///
    /// This is the whole wire→domain translation for decision #14's grouping rule, and it
    /// is deliberately one total function of one declared field **plus one declared
    /// profile**: no handle-range test, no `processName` sniffing, no dup-graph
    /// inference, and — the part that was missing — no assumption about which OS wrote
    /// the field.
    ///
    /// # Errors
    ///
    /// [`ClientKindRuleUnknown`] when this profile has no [`ClientKindRule`]. The caller
    /// owes the guest a refusal; it must never substitute a kind.
    pub const fn client_kind_from_process_id(
        self,
        process_id: u32,
    ) -> Result<ClientKind, ClientKindRuleUnknown> {
        match self.client_kind_rule() {
            Some(ClientKindRule::KernelPidSentinel) => Ok(if process_id == KERNEL_PID {
                ClientKind::Kernel
            } else {
                ClientKind::User { pid: process_id }
            }),
            None => Err(ClientKindRuleUnknown {
                guest_os: self,
                process_id,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `KERNEL_PID` sentinel is the ONLY thing that makes a client kernel-privileged
    /// **under the Linux profile**, and every other value — including the numerically
    /// adjacent ones and the handle-like values that seeded the old mis-reading — is a
    /// user pid. Kills the `==`→`!=`, `==`→`>=` and constant mutants in one predicate.
    ///
    /// ★ This is the pre-2026-07-29 behaviour, unchanged, asserted through the new seam:
    /// making the OS explicit must cost the only guest we can run exactly nothing.
    #[test]
    fn only_the_kernel_sentinel_decodes_to_a_kernel_client_on_linux() {
        assert_eq!(
            GuestOs::Linux.client_kind_from_process_id(0xFFFF_FFFF),
            Ok(ClientKind::Kernel)
        );
        for pid in [
            0u32,
            1,
            0x0000_dd13, // process A, measured
            0x0000_dd14, // process B, measured
            0xFFFF_FFFE, // adjacent below the sentinel
            0x7FFF_FFFF, // sign-bit boundary
            0xc1d0_0069, // ★ the UVM session's HANDLE — not its processID
        ] {
            assert_eq!(
                GuestOs::Linux.client_kind_from_process_id(pid),
                Ok(ClientKind::User { pid }),
                "processID {pid:#x} declares a user client",
            );
        }
    }

    /// ★★★ **The fold, refused.** The same bytes that classify on Linux must produce a
    /// typed refusal on Windows — and in particular the sentinel itself must NOT come
    /// back as `Kernel`, because "the sentinel still means kernel, we just also accept
    /// pids" is the most tempting wrong rule available.
    #[test]
    fn windows_refuses_every_process_id_rather_than_classifying_one() {
        for pid in [0u32, 1, 0x0000_dd13, 0xFFFF_FFFE, 0xFFFF_FFFF] {
            assert_eq!(
                GuestOs::Windows.client_kind_from_process_id(pid),
                Err(ClientKindRuleUnknown {
                    guest_os: GuestOs::Windows,
                    process_id: pid,
                }),
                "processID {pid:#x} is unclassifiable on a guest whose rule we do not have",
            );
        }
        assert_eq!(GuestOs::Windows.client_kind_rule(), None);
    }

    /// ★ Non-vacuity for the test above: the Linux profile answers for **every** one of
    /// those same inputs. Without this, "Windows refuses" would also hold of a function
    /// that refused unconditionally.
    #[test]
    fn the_windows_refusal_is_about_the_profile_and_not_about_the_inputs() {
        for pid in [0u32, 1, 0x0000_dd13, 0xFFFF_FFFE, 0xFFFF_FFFF] {
            assert!(
                GuestOs::Linux.client_kind_from_process_id(pid).is_ok(),
                "the Linux profile classifies {pid:#x}; only the OS differs",
            );
        }
    }

    /// An unrecognised profile name REFUSES. `""`, a typo and a case variant are the
    /// three shapes an operator actually produces, and only the third is accepted.
    #[test]
    fn an_unknown_guest_os_name_refuses_rather_than_defaulting_to_linux() {
        for bad in ["", "windwos", "linux ", "lin", "unix", "wddm", "l"] {
            assert_eq!(
                GuestOs::from_config_name(bad),
                Err(UnknownGuestOsName),
                "{bad:?} must not resolve to a profile",
            );
        }
        assert_eq!(GuestOs::from_config_name("linux"), Ok(GuestOs::Linux));
        assert_eq!(GuestOs::from_config_name("LiNuX"), Ok(GuestOs::Linux));
        assert_eq!(GuestOs::from_config_name("windows"), Ok(GuestOs::Windows));
        assert_eq!(GuestOs::from_config_name("WINDOWS"), Ok(GuestOs::Windows));
    }

    /// `config_name` round-trips through `from_config_name` for every variant — so the
    /// diagnostic string and the door agree, and a new variant cannot be spelled one way
    /// in an error and another in a flag.
    #[test]
    fn every_profile_name_round_trips() {
        for os in [GuestOs::Linux, GuestOs::Windows] {
            assert_eq!(GuestOs::from_config_name(os.config_name()), Ok(os));
            assert!(
                GUEST_OS_CONFIG_NAMES.contains(&os.config_name()),
                "{os:?} is not listed in GUEST_OS_CONFIG_NAMES, so an operator-facing \
                 error would omit a name the parser accepts",
            );
        }
        assert_eq!(
            GUEST_OS_CONFIG_NAMES.len(),
            2,
            "★ NON-VACUITY: the name list must cover every variant, or the loop above is \
             about a subset",
        );
    }

    /// The configuration default is Linux, and it is the only guest we can run.
    ///
    /// Pinned rather than assumed: `Default` is the one place a `GuestOs` appears without
    /// somebody naming it, so a change to it is a change to what an unconfigured
    /// deployment serves, and that must not be a silent diff.
    #[test]
    fn the_configuration_default_is_linux() {
        assert_eq!(GuestOs::default(), GuestOs::Linux);
    }

    /// ★★ **Characterisation, not endorsement** — the second gate from the module docs
    /// §4, pinned so the day Orin-class hardware becomes a target this test is what
    /// fails.
    ///
    /// On `IsT234DorBetter` silicon the guest's RM never writes `processID` at all
    /// (`ogkm-580: rpc.h:53,57` / `ogkm-610: rpc.h:53,57`), so every client — kernel and
    /// user — declares `0`. Today's Linux rule answers `User { pid: 0 }` for all of them,
    /// which on that hardware collapses the whole guest into one `ClientKind`. That is
    /// recorded here rather than fixed, because our target is GA106 and inventing a rule
    /// for hardware we have never observed is the mistake §3 refuses for Windows.
    #[test]
    fn a_zero_process_id_is_a_user_client_today_and_that_is_wrong_on_t234d() {
        assert_eq!(
            GuestOs::Linux.client_kind_from_process_id(0),
            Ok(ClientKind::User { pid: 0 }),
            "documented, not endorsed: on T234D-or-better EVERY client declares 0",
        );
    }
}
