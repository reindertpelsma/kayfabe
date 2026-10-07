// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! `kf-gop-export <out.efi>` — write kayfabe's embedded boot-display GOP driver
//! (`kf_gop_image::KF_GOP_EFI`) to a file, so it can be signed with a per-install Secure Boot key
//! (`sbsign`) and handed back to kf3 as `gop-efi=<signed.efi>` (2026-10-04, branch `v3-windows`,
//! `docs/OWNER_RULINGS.md` §K; `scripts/bench/windows/win_vm.sh`).
//!
//! Prints one line, `KF_GOP_EXPORT target=<triple> bytes=<n> out=<path>`. The file is the embedded
//! bytes exactly; kf3 accepts a signed copy only if it is these bytes plus a certificate table.
//! Refuses to overwrite an existing file.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [out] = args.as_slice() else {
        eprintln!("usage: kf-gop-export <out.efi>");
        return ExitCode::from(2);
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out);
    let written = file.and_then(|mut f| {
        f.write_all(kf_gop_image::KF_GOP_EFI)?;
        f.sync_all()
    });
    match written {
        Ok(()) => {
            println!(
                "KF_GOP_EXPORT target={} bytes={} out={}",
                kf_gop_image::TARGET,
                kf_gop_image::KF_GOP_EFI.len(),
                std::path::Path::new(out).display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "kf-gop-export: writing {}: {e}",
                std::path::Path::new(out).display()
            );
            ExitCode::FAILURE
        }
    }
}
