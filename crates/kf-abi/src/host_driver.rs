//! ★★★ The **host** driver's version — the axis that does not exist, made LOUD.
//!
//! # 0. What this module is for
//!
//! `four_axes_of_variation.md` states the product property: the guest driver version and
//! the host driver version are **disjoint**. Mode 2 does not replay the guest's ioctls on
//! the host; it reconstructs intent and re-issues it, so the guest's driver version is the
//! guest's business and the host's is ours. `host_driver_version_pin.md` is the note that
//! states the property and — this is the part that matters here — states **what is
//! actually true today**:
//!
//! - the **guest** axis is built: [`crate::DriverVersion`] is a full `(major, minor,
//!   patch)` triple with a table per version and a loud floor when there is no table;
//! - the **host** axis is **not built**. `kayfabe-isolate-host` encodes every host-side
//!   RM parameter block with a **const-size, version-free** encoder — `…::SIZE` buffers
//!   and `encode_into`, used unconditionally. Those encoders were transcribed from one
//!   driver.
//!
//! ⊘ **This module does NOT build the missing axis**, and that is a decision rather than
//! an omission. There is exactly one host driver available to this project, so a host
//! version axis would be a mechanism with no red available to it: every table but one
//! would be unexercised, and every test over it would pass for the wrong reason. What the
//! module does instead is convert a **silent pin into a named refusal** — the same move
//! as `RefusingRam` and `GspFault::GuestRam`. A wrong struct offset is *plausible*: the
//! ioctl succeeds, the channel lands on a different runlist, and the symptom arrives
//! several layers away. A refusal is not plausible, so it surfaces at the rung that made
//! it.
//!
//! # 1. What the encoders are pinned to, and how that was established
//!
//! `crate::submit`'s own module docs record the provenance: *"everything here is read off
//! the bench's own driver, `ogkm-580: 580.159.04`"*, and the central struct cannot come
//! from the generator because the two vendored trees disagree —
//! `ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` has an
//! `hHandleVASpace` at +32 that `ogkm-580: .../alloc_channel.h:296-342` does not, so every
//! field from +32 onward differs by four bytes between them. `engineType` sits at +128 on
//! one tree and +132 on the other.
//!
//! The floor is **not** "major 580", and that is the second reading behind this module.
//! `crate::versions` records `NVOS46_PARAMETERS` growing 56 → 64 bytes at **580.65.06**
//! and keeps *both* encodings behind [`crate::versions::MapDmaWire`] for the guest.
//! `kayfabe-isolate-host` has no such selector: its `map_memory_dma` writes a
//! `Nvos46Parameters::SIZE` buffer — the 64-byte, post-580.65.06 form — unconditionally.
//! So the encoders are pinned to the *interval* `[580.65.06, 581)`, not to a major.
//!
//! Both facts are **source readings** (`ogkm-580` / `ogkm-610` at the tags vendored under
//! `research_clones/`) plus a read of this tree's own encoders. Nothing here is a
//! statement about what a mismatched host does at runtime — no host other than
//! 580.159.04 has been run against this code.
//!
//! # 2. Where the version comes from
//!
//! [`crate::bringup::NV_ESC_CHECK_VERSION_STR`] in its **query** form (`cmd = '2'`), on
//! `/dev/nvidiactl`. `RmPerformVersionCheck` answers that form by copying
//! `NV_VERSION_STRING` into the reply and returning `NV_OK`
//! (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:1066-1071`), so what comes back is
//! the RM's own version string — `"580.159.04"` on the bench.
//!
//! ★ Three sources were available and two are unusable **at the place the check has to
//! happen**:
//!
//! | source | why not |
//! |---|---|
//! | `/proc/driver/nvidia/version` | unreachable. The isolate child runs inside a `pivot_root`ed sandbox whose scratch root is mounted **over `/proc`** and whose old root is detached (`kayfabe_linux_raw::sandbox_unsafe`, `SCRATCH = "/proc"`). There is no `/proc` inside the process that issues the encoded ioctls. |
//! | the kernel module version (`modinfo`, sysfs) | describes the module on disk, not the frontend this session is bound to, and needs a filesystem the sandbox does not have either. |
//! | ★ `NV_ESC_CHECK_VERSION_STR` | the **same descriptor** the encoders are about to be used on. It answers inside the sandbox, and it names the RM that will parse our structs rather than a module that happens to be installed. |
//!
//! ⚠ **Unreadable is a refusal, not a default.** [`HostDriverRefusal::Unreadable`] exists
//! precisely so that "the frontend did not answer" cannot collapse into "assume 580". The
//! same applies to a reply this module cannot parse
//! ([`HostDriverRefusal::Unparsable`]) — an unrecognised string is *less* evidence than no
//! string, not more.
//!
//! # 3. What this deliberately does not have
//!
//! **No override.** No environment variable, no flag, no "force" argument. A refusal that
//! can be switched off is a refusal that will be switched off by whoever meets it at 2am,
//! and what is behind it is silent memory corruption on the host's own GPU. The way past
//! it is a host-side version table — named as the follow-on in
//! `host_driver_version_pin.md` §5, not built here.

use std::fmt;

// =====================================================================================
// The pin
// =====================================================================================

/// The major version `kayfabe-isolate-host`'s encoders were transcribed from.
///
/// `crate::submit` §"Provenance": read off `ogkm-580: 580.159.04`.
pub const ENCODED_FOR_MAJOR: u16 = 580;

/// The `(minor, patch)` at which the pinned interval **opens**, and the reason it is not
/// the whole major.
///
/// `NVOS46_PARAMETERS` grows 56 → 64 bytes here (`crate::versions`, and
/// `docs/reference/nvidia_abi_oracles.md` F1). The guest side keeps both encodings behind
/// [`crate::versions::MapDmaWire`]; the host side writes the 64-byte form unconditionally,
/// so a host *below* this point would be handed a struct eight bytes longer than the one
/// its frontend sizes.
pub const ENCODED_FOR_FLOOR: (u16, u16) = (65, 6);

/// The major at which `NV_CHANNEL_ALLOC_PARAMS` shifts — the delta the refusal names.
///
/// `ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` inserts
/// `hHandleVASpace` at +32; `ogkm-580: .../alloc_channel.h:296-342` has no such field.
/// Everything from +32 onward moves by four bytes, `engineType` included.
pub const CHANNEL_PARAMS_SHIFT_MAJOR: u16 = 610;

/// The **host** driver's version, as the host frontend reported it.
///
/// ★ A distinct type from [`crate::DriverVersion`], which is documented as *"a guest
/// driver version"*. They are the same three numbers about **two independent machines**,
/// and the whole product property is that they need not agree — so the type system is
/// told, and a guest-side triple cannot be handed to the host-side check by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostDriverVersion {
    /// Major (e.g. 580).
    pub major: u16,
    /// Minor (e.g. 159).
    pub minor: u16,
    /// Patch (e.g. 4 — the driver writes it zero-padded, `"04"`).
    pub patch: u16,
}

impl fmt::Display for HostDriverVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{:02}", self.major, self.minor, self.patch)
    }
}

impl HostDriverVersion {
    /// Parse the frontend's `versionString`.
    ///
    /// Strict: exactly three decimal components, nothing else. A looser parser is the
    /// wrong trade here — a string we half-understand is the input to a decision about
    /// **struct offsets**, and the honest answer to "this does not look like a version" is
    /// [`HostDriverRefusal::Unparsable`], never a best guess at the major.
    #[must_use]
    pub fn parse(reported: &str) -> Option<Self> {
        let mut parts = reported.trim().split('.');
        let mut next = || -> Option<u16> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse::<u16>().ok()
        };
        let major = next()?;
        let minor = next()?;
        let patch = next()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    /// Is this host inside the interval the host-side encoders were transcribed for?
    ///
    /// `[580.65.06, 581)` — see [`ENCODED_FOR_MAJOR`] and [`ENCODED_FOR_FLOOR`] for the two
    /// readings that set each end.
    #[must_use]
    pub fn is_encoded_for(self) -> bool {
        self.major == ENCODED_FOR_MAJOR && (self.minor, self.patch) >= ENCODED_FOR_FLOOR
    }
}

// =====================================================================================
// The refusal
// =====================================================================================

/// Why the host driver in front of us must not be handed these encoders' output.
///
/// Every arm's [`fmt::Display`] says three things in one line: **what the host is**, **what
/// the encoders emit**, and **that we are refusing rather than encoding wrong offsets**.
/// That shape is the point — the reader of a bring-up log has to be able to tell this apart
/// from a driver error without opening the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostDriverRefusal {
    /// The frontend did not answer the version query at all.
    ///
    /// ⚠ Its own arm rather than a `None` folded into "not encoded for", because the two
    /// need different fixes and because this is the arm that must never silently mean 580.
    Unreadable,
    /// The frontend answered something that is not a `major.minor.patch` triple.
    Unparsable {
        /// Exactly what came back, so a log says what confused us.
        reported: String,
    },
    /// A perfectly readable host driver, outside the interval these encoders cover.
    NotEncodedFor {
        /// The host driver's own version.
        found: HostDriverVersion,
    },
}

/// The interval, rendered for a message. Written from the constants so a change to the pin
/// changes what the refusal says.
fn encoded_interval() -> String {
    let (minor, patch) = ENCODED_FOR_FLOOR;
    format!(
        "{ENCODED_FOR_MAJOR}.{minor}.{patch:02}-era struct layouts \
         (the interval [{ENCODED_FOR_MAJOR}.{minor}.{patch:02}, {}.0.00))",
        ENCODED_FOR_MAJOR + 1
    )
}

impl fmt::Display for HostDriverRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let emit = encoded_interval();
        match self {
            Self::Unreadable => write!(
                f,
                "the host driver did not answer NV_ESC_CHECK_VERSION_STR, so there is no \
                 host driver version to check; kayfabe-isolate-host's encoders emit {emit}, \
                 and an unread version is not evidence of one; refusing rather than \
                 assuming {ENCODED_FOR_MAJOR}."
            ),
            Self::Unparsable { reported } => write!(
                f,
                "the host driver reported version {reported:?}, which is not a \
                 major.minor.patch triple; kayfabe-isolate-host's encoders emit {emit}, and \
                 a string we cannot compare is not evidence of a match; refusing rather \
                 than assuming {ENCODED_FOR_MAJOR}."
            ),
            Self::NotEncodedFor { found } => {
                write!(
                    f,
                    "host driver is {found}; kayfabe-isolate-host's encoders emit {emit}"
                )?;
                if found.major >= CHANNEL_PARAMS_SHIFT_MAJOR {
                    write!(
                        f,
                        " (NV_CHANNEL_ALLOC_PARAMS gains hHandleVASpace at +32 from \
                         {CHANNEL_PARAMS_SHIFT_MAJOR}, shifting engineType from +128 to \
                         +132 — a channel encoded at {ENCODED_FOR_MAJOR} offsets and read \
                         by a {CHANNEL_PARAMS_SHIFT_MAJOR} driver names a different \
                         runlist)"
                    )?;
                } else if found.major == ENCODED_FOR_MAJOR {
                    let (minor, patch) = ENCODED_FOR_FLOOR;
                    write!(
                        f,
                        " (NVOS46_PARAMETERS is 56 bytes below \
                         {ENCODED_FOR_MAJOR}.{minor}.{patch:02} and 64 bytes at and above \
                         it, and the host-side map_memory_dma writes the 64-byte form \
                         unconditionally)"
                    )?;
                } else {
                    write!(
                        f,
                        " (no layout delta for {} is transcribed anywhere in this tree, so \
                         the offsets we would send are unchecked rather than wrong-and-known)",
                        found.major
                    )?;
                }
                write!(f, "; refusing rather than encoding wrong offsets.")
            }
        }
    }
}

impl std::error::Error for HostDriverRefusal {}

/// ★★★ The gate. Given whatever the host frontend reported, either a host driver these
/// encoders may be pointed at, or the named reason they may not.
///
/// `None` is the "the query failed" case and is deliberately part of this function's input
/// rather than handled by the caller: the caller is the one place that could turn it into a
/// default, so it never gets the chance.
///
/// # Errors
/// [`HostDriverRefusal`], whose `Display` is the message to put in the log.
pub fn check(reported: Option<&str>) -> Result<HostDriverVersion, HostDriverRefusal> {
    let Some(reported) = reported else {
        return Err(HostDriverRefusal::Unreadable);
    };
    let Some(found) = HostDriverVersion::parse(reported) else {
        return Err(HostDriverRefusal::Unparsable {
            reported: reported.to_string(),
        });
    };
    if found.is_encoded_for() {
        Ok(found)
    } else {
        Err(HostDriverRefusal::NotEncodedFor { found })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The bench's own host driver is accepted.
    ///
    /// The literal is the string `RmPerformVersionCheck`'s query arm returns on the box
    /// this project builds on — RTX 3090, driver 580.159.04, the same string
    /// `/proc/driver/nvidia/version` prints there. Written as a **literal**, never derived
    /// from [`ENCODED_FOR_MAJOR`]: a test that recomputes the constant under test asserts
    /// nothing.
    #[test]
    fn the_benchs_own_host_driver_is_accepted() {
        let v = check(Some("580.159.04")).expect("the bench's driver must be accepted");
        assert_eq!(v.major, 580);
        assert_eq!(v.minor, 159);
        assert_eq!(v.patch, 4);
    }

    /// ★★★ The refusal fires on the other vendored tree, and its text names the delta.
    ///
    /// 610.43.02 is not hypothetical — it is `research_clones/ogkm/`, the tree the
    /// generator's own record compares against, and the tree whose `NV_CHANNEL_ALLOC_PARAMS`
    /// differs from ours at +32.
    #[test]
    fn the_other_vendored_tree_is_refused_by_name() {
        let e = check(Some("610.43.02")).expect_err("610 must not be encoded for");
        assert_eq!(
            e,
            HostDriverRefusal::NotEncodedFor {
                found: HostDriverVersion {
                    major: 610,
                    minor: 43,
                    patch: 2
                }
            }
        );
        let msg = e.to_string();
        // Three separate obligations, asserted separately so a message that loses one of
        // them fails on that one.
        assert!(msg.contains("610.43.02"), "names the host driver: {msg}");
        assert!(
            msg.contains("NV_CHANNEL_ALLOC_PARAMS") && msg.contains("+32"),
            "names the concrete delta: {msg}"
        );
        assert!(
            msg.contains("refusing rather than encoding wrong offsets"),
            "says what it is doing instead: {msg}"
        );
    }

    /// ★★ The floor is inside the major, not at its edge — the `NVOS46` growth.
    ///
    /// The pair straddles 580.65.06 by one patch. A predicate that had collapsed to
    /// `major == 580` passes the first of these and fails the second, so the second is the
    /// one that carries the reading.
    #[test]
    fn the_nvos46_floor_splits_the_580_line() {
        assert!(check(Some("580.65.06")).is_ok(), "the floor itself is in");
        let e = check(Some("580.65.05")).expect_err("one patch below the floor is out");
        let msg = e.to_string();
        assert!(msg.contains("580.65.05"), "names the host driver: {msg}");
        assert!(
            msg.contains("NVOS46_PARAMETERS") && msg.contains("56 bytes"),
            "names the delta that applies to THIS side of the floor: {msg}"
        );
    }

    /// ★ A neighbouring major with no transcribed delta is still refused, and says so
    /// honestly rather than inventing a reason.
    ///
    /// 575.64.05 and 590.44.01 are both real tags named in `four_axes_of_variation.md` §3;
    /// neither is vendored here, so the message must claim no knowledge of their layout.
    #[test]
    fn a_major_with_no_transcribed_delta_refuses_without_inventing_one() {
        for tag in ["575.64.05", "590.44.01", "595.84.00"] {
            let msg = check(Some(tag)).expect_err("not encoded for").to_string();
            assert!(msg.contains(tag), "names the host driver: {msg}");
            assert!(
                msg.contains("no layout delta for"),
                "claims no knowledge instead of a delta it does not have: {msg}"
            );
            assert!(
                !msg.contains("NV_CHANNEL_ALLOC_PARAMS"),
                "must not borrow 610's delta for a version it was not read from: {msg}"
            );
        }
    }

    /// ★★★ Unreadable is a REFUSAL, and it is a *different* refusal from every other one.
    ///
    /// This is the arm the whole module exists for: the pre-existing code path read the
    /// version with `.unwrap_or_default()`, so a frontend that did not answer produced the
    /// empty string and bring-up continued. Both no-answer and a nonsense answer must
    /// refuse, and must not be confusable with each other.
    #[test]
    fn an_unread_or_unparsable_version_never_means_580() {
        assert_eq!(check(None).unwrap_err(), HostDriverRefusal::Unreadable);
        // The exact value the old `.unwrap_or_default()` produced.
        assert_eq!(
            check(Some("")).unwrap_err(),
            HostDriverRefusal::Unparsable {
                reported: String::new()
            }
        );
        assert!(
            check(None).unwrap_err() != check(Some("")).unwrap_err(),
            "no-answer and a bad answer are different findings"
        );
        for bad in [
            "580",
            "580.159",
            "580.159.04.1",
            "580.x.04",
            "v580.159.04",
            "",
        ] {
            let msg = check(Some(bad)).unwrap_err().to_string();
            assert!(
                msg.contains("refusing rather than assuming 580"),
                "{bad:?} must refuse, not default: {msg}"
            );
        }
    }

    /// ★ Rendering round-trips the driver's own zero padding, so a log line can be grepped
    /// against `/proc/driver/nvidia/version` verbatim.
    #[test]
    fn a_rendered_version_matches_the_string_the_driver_prints() {
        let v = HostDriverVersion::parse("580.159.04").expect("parses");
        assert_eq!(v.to_string(), "580.159.04");
        let v = HostDriverVersion::parse("610.43.02").expect("parses");
        assert_eq!(v.to_string(), "610.43.02");
    }
}
