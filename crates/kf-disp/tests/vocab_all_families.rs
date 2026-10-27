// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! Every display family's method vocabulary must resolve from the derived tables for every
//! derived driver version. `display=on` refuses to realize otherwise, and a rented Turing box
//! (TU106, lease 57882297, 2026-10-27) showed that Turing could not:
//! `NVC57E_SET_SEMAPHORE_CONTROL_PAYLOAD_SIZE is not derived`. Turing's window class (C57E) has a
//! 32-bit-only semaphore with one context DMA, one control word and an ACQUIRE and a RELEASE
//! value; the 64-bit payload bit, the release mode and the separate acquire semaphore are C67E
//! and later (`crates/kf-disp/data/classes-*.tsv`).

use kf_disp::class;
use kf_disp::engine::Vocab;
use kf_disp::model::Classes;
use kf_disp::regs::Regs;

#[test]
fn every_family_resolves_its_vocabulary_for_every_derived_version() {
    let mut failures = Vec::new();
    for version in ["580.65.06", "580.159.04"] {
        let t = class::for_version(version).expect("derived classes");
        for row in kf_chip::display::ALL {
            // Blackwell's display plane lives on its own branches and is not realized from this
            // tree (NVCA7D has no context-DMA notifier); it is not this test's subject.
            if row.chips.starts_with("GB") {
                continue;
            }
            let r = Regs::for_ip(version, row.ip_version).expect("derived registers");
            if let Err(e) = Vocab::resolve(t, &Classes::of(row), &r) {
                failures.push(format!("{version} {}: {}", row.chips, e.0));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "unresolved vocabularies: {failures:#?}"
    );
}
