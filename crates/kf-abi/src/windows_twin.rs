// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **A Windows driver build, mapped to the driver-matrix Linux tag that is the same source snapshot**
//! (2026-10-04, branch `v3-windows`, runbook C2; `docs/design/V3_WINDOWS_DISCOVERY.md`).
//!
//! kf3 keys every guest-facing layout on the exact ogkm tag (`tools/drivermatrix/tags.txt`). A
//! Windows guest's RM is built from the same tree, but it names itself by its Windows build
//! (`NV_BUILD_NAME` of `nvBldVer.h`'s Windows block, e.g. `580.88`), never by the Linux tag. Each
//! tag's own `nvBldVer.h` states both builds and both changelists
//! ([`crate::generated::windows_twins`], read by `tools/drivermatrix/windows_twins.py`), and the guest
//! states its changelist on the wire (fn 1's `guestClNum`, [`crate::guestsysinfo`]).
//!
//! So a Windows build is accepted as a tag's twin ONLY when (a) its fn-1 `guestDriverVersion` AND
//! `guestVersion` are that tag's Windows `NV_BUILD_NAME` and `NV_BUILD_BRANCH_VERSION`, (b) the tag's
//! Windows and Linux changelists are one number, and (c) the guest's `guestClNum` is that number or 0.
//! Same-branch twins with different changelists (e.g. Windows 582.53 at 37799871 vs Linux 580.159.04
//! at 37889135) stay refused, by name: NVIDIA has moved ABI inside a branch before
//! (`V3_DRIVER_MATRIX.md`).
//!
//! ⊘ CORRECTED 2026-10-04, by the first Windows run on `vwin` (RTX 3060, kf3 `b98bdbec`): the
//! runbook planned to check the changelist ON THE WIRE, and the Windows 580.88 guest sends
//! `guestClNum=0` — its title is `DVSReal r580_78 580.88 DVS-Applications`, a build without the
//! buildmeister define, whose `NV_BUILD_CHANGELIST_NUM` falls back to `NV_BUILD_CL` (nvBldVer.h). The
//! Linux `.run` builds send 0 as well (`Private` title, same run). So 0 is "no changelist stated",
//! the match is carried by the two strings (a), and (b) is read from the tag's own header.
//!
//! ⚠ Equal changelists say the two builds share a source snapshot; they do not say the binaries are
//! byte-identical (the Windows 580.88 GSP firmware's `.fwversion` is `580.65.05`, Linux 580.65.06's
//! is `580.65.06`). That is an inference this mapping rests on, and the discovery doc lists it.

use crate::DriverVersion;
use crate::generated::windows_twins::{WINDOWS_TWINS, WindowsTwin};

/// Why a reported version is not accepted as a driver-matrix tag's Windows twin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwinRefusal {
    /// No driver-matrix tag's Windows block names this build.
    NoTwin,
    /// The tag's Windows and Linux builds have different changelists: not one source snapshot.
    ChangelistsDiffer(&'static WindowsTwin),
    /// The guest's `guestVersion` is not the twin's Windows `NV_BUILD_BRANCH_VERSION`.
    GuestBranch {
        /// The twin the name matched.
        twin: &'static WindowsTwin,
        /// What the guest sent.
        guest_branch: String,
    },
    /// The guest's `guestClNum` is neither 0 nor the twin's changelist.
    GuestChangelist {
        /// The twin the name matched.
        twin: &'static WindowsTwin,
        /// What the guest sent.
        guest_cl: u32,
    },
}

impl core::fmt::Display for TwinRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TwinRefusal::NoTwin => write!(f, "no driver-matrix tag's Windows build has this name"),
            TwinRefusal::ChangelistsDiffer(t) => write!(
                f,
                "Windows {} (CL {}, {}) is the Windows build of Linux {} (CL {}, {}), but the \
                 changelists differ, so they are not one source snapshot (refused; NVIDIA moves ABI \
                 inside a branch)",
                t.win_name, t.win_cl, t.win_branch, t.linux_tag, t.linux_cl, t.linux_branch
            ),
            TwinRefusal::GuestBranch { twin, guest_branch } => write!(
                f,
                "Windows {} is Linux {}'s twin with branch version {}, but the guest's guestVersion is \
                 {guest_branch:?}",
                twin.win_name, twin.linux_tag, twin.win_branch
            ),
            TwinRefusal::GuestChangelist { twin, guest_cl } => write!(
                f,
                "Windows {} is Linux {}'s twin at CL {}, but the guest's guestClNum is {guest_cl}",
                twin.win_name, twin.linux_tag, twin.win_cl
            ),
        }
    }
}

/// The driver-matrix Linux tag whose Windows build a guest's fn-1 identity names: `reported` is
/// `guestDriverVersion`, `guest_branch` is `guestVersion`, `guest_cl` is `guestClNum`
/// (`crate::guestsysinfo::GuestIdentity`).
///
/// # Errors
/// A [`TwinRefusal`], by name.
pub fn linux_twin(
    reported: &str,
    guest_branch: &str,
    guest_cl: u32,
) -> Result<(DriverVersion, &'static WindowsTwin), TwinRefusal> {
    let twin = WINDOWS_TWINS
        .iter()
        .find(|t| t.win_name == reported.trim())
        .ok_or(TwinRefusal::NoTwin)?;
    if twin.win_cl != twin.linux_cl {
        return Err(TwinRefusal::ChangelistsDiffer(twin));
    }
    if guest_branch.trim() != twin.win_branch {
        return Err(TwinRefusal::GuestBranch {
            twin,
            guest_branch: guest_branch.trim().to_owned(),
        });
    }
    // 0 = a build without the buildmeister define states no changelist (see the module docs).
    if guest_cl != 0 && guest_cl != twin.win_cl {
        return Err(TwinRefusal::GuestChangelist { twin, guest_cl });
    }
    let v = DriverVersion::parse(twin.linux_tag).ok_or(TwinRefusal::NoTwin)?;
    Ok((v, twin))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The identity the Windows 580.88 guest sent at fn 1 on `vwin`, 2026-10-04 (kf3 `b98bdbec`):
    /// `guestDriverVersion="580.88" guestVersion="r580_78-7" guestClNum=0` — Linux 580.65.06's twin.
    #[test]
    fn the_observed_windows_580_88_identity_is_linux_580_65_06() {
        let (v, t) = linux_twin("580.88", "r580_78-7", 0).unwrap();
        assert_eq!(v, DriverVersion::parse("580.65.06").unwrap());
        assert_eq!((t.win_cl, t.linux_cl), (36_308_443, 36_308_443));
        assert!(
            linux_twin("580.88", "r580_78-7", 36_308_443).is_ok(),
            "a buildmeister build stating its changelist"
        );
    }

    #[test]
    fn a_wrong_branch_a_wrong_changelist_and_a_different_snapshot_are_refused_by_name() {
        assert!(matches!(
            linux_twin("580.88", "r580_78-6", 0),
            Err(TwinRefusal::GuestBranch { .. })
        ));
        assert!(matches!(
            linux_twin("580.88", "r580_78-7", 1),
            Err(TwinRefusal::GuestChangelist { guest_cl: 1, .. })
        ));
        let e = linux_twin("582.53", "r582_49-2", 0).unwrap_err();
        assert!(matches!(e, TwinRefusal::ChangelistsDiffer(t) if t.linux_tag == "580.159.04"));
        assert!(e.to_string().contains("not one source snapshot"));
        assert_eq!(linux_twin("999.99", "x", 0), Err(TwinRefusal::NoTwin));
    }

    /// Every row names a tag of the driver matrix (`generated::matrix::MEASURED`, which is
    /// `tags.txt`; a tag there may still be refused by `versions::table_for`, e.g. 615.71.09), and a
    /// Windows build name is never two tags' twin.
    #[test]
    fn every_row_is_a_matrix_tag_and_twin_names_are_unique() {
        assert_eq!(
            WINDOWS_TWINS.len(),
            crate::generated::matrix::MEASURED.len()
        );
        for t in WINDOWS_TWINS {
            let v = DriverVersion::parse(t.linux_tag).expect("a tag parses");
            assert!(
                crate::generated::matrix::MEASURED.contains(&v),
                "{} is a matrix tag",
                t.linux_tag
            );
            assert_eq!(
                WINDOWS_TWINS
                    .iter()
                    .filter(|o| o.win_name == t.win_name)
                    .count(),
                1,
                "{}",
                t.win_name
            );
        }
    }
}
