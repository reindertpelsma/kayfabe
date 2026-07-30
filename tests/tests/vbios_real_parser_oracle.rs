//! **The differential oracle for the synthetic VBIOS: NVIDIA's own parser, compiled.**
//!
//! # The hole this closes, in the generator author's own words
//!
//! `kayfabe_abi::vbios` builds a ROM image; `kayfabe_abi::vbios::tests` reads it back.
//! Both were written by reading the same two C files, and the module doc says what that
//! costs: *"The test oracle is a transcription, not independent. If I misread the C,
//! builder and parser are wrong the same way and stay green."*
//!
//! A transcribed parser cannot detect a **shared misreading**. Only something that did
//! not come from our reading of the C can — and until now the only such thing was a full
//! guest boot: slow, needs KVM and a GPU, whole-system, and it validates one image rather
//! than a property.
//!
//! So this suite runs the image through the **actual driver code**:
//!
//! ```text
//! kgspExtractVbiosFromRom_TU102   (ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_vbios_tu102.c)
//!   -> kgspParseFwsecUcodeFromVbiosImg_IMPL (ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp_fwsec.c)
//!   -> s_vbiosPatchInterfaceData           (ogkm-580: src/nvidia/src/kernel/gpu/gsp/arch/turing/kernel_gsp_frts_tu102.c)
//! ```
//!
//! compiled UNMODIFIED, against those trees' own headers, and served the image through a
//! stubbed `GPU_REG_RD32` at the **PROM window** — which is exactly how a real device is
//! read. There is no transcription anywhere in the path.
//!
//! ★ The third stage is a **second plane**, and it was added because a stock guest proved
//! the first two are not enough: an image the parser accepts completely can still stop
//! adapter init at `s_prepareForFwsec_TU102` with *"failed to find required interface entry
//! for FWSEC cmd 0x15"*. See section 8 below for how it is compiled and what it still
//! cannot see.
//!
//! # Licensing
//!
//! The parser sources are NVIDIA's, dual MIT / GPL-2.0 (`SPDX-License-Identifier: MIT`
//! in each header). Compiling a slice of them for testing is within the MIT grant.
//! **Nothing from those trees is vendored into this repository**: `tests/build.rs`
//! `#include`s them out of the existing checkout under
//! `/workspace/nvidia-gpu-passthrough/research_clones/` (relocatable via `KAYFABE_OGKM_580`
//! / `KAYFABE_OGKM_610`) and skips loudly rather than substituting a copy.
//!
//! # How it skips, and what that is worth
//!
//! Every test here prints `VBIOS-ORACLE-GATE: RAN <name>` or
//! `VBIOS-ORACLE-GATE: SKIPPED <name> — …` to stderr, in **both** arms, and CI counts
//! RAN+SKIPPED against a floor. That is the KVM gate's shape, and its known limit applies
//! here too and is stated rather than glossed: **GitHub's runners do not have the vendored
//! trees**, so on that runner this suite is counted and never passes. It is a
//! developer-box and bench gate. The floor is what stops it from silently vanishing from
//! both.
//!
//! # What this suite is NOT
//!
//! It says nothing about whether the *emulated device* serves the image correctly — only
//! about whether the image is one the real parser accepts, and what the real parser makes
//! of it. The register/PROM plumbing is a different seam.

use kayfabe_abi::generated::vbios::{
    BCRT30_RSA3K_SIG_SIZE, FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
    FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB,
    FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER, FALCON_UCODE_DESC_V3_SIZE_44,
    FalconApplicationInterfaceDmemMapperV3, FalconApplicationInterfaceEntryV1,
    FalconApplicationInterfaceHeaderV1,
};
use kayfabe_abi::vbios::{
    FWSEC_CMD_BUFFER_MIN, INTERFACE_ENTRY_COUNT, UCODE_ALIGN, UCODE_MARKER, VbiosWire, build,
    profile_for_device_id,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// ===========================================================================================
// The gate
// ===========================================================================================

/// The oracles this build has, as `(tag, path)`. Empty when no vendored tree was found.
///
/// `option_env!` and not `env!`: the whole point is that a machine without the trees still
/// builds and tests everything else.
fn oracles() -> Vec<(&'static str, &'static str)> {
    [
        ("ogkm-580.159.04", option_env!("KAYFABE_VBIOS_ORACLE_580")),
        (
            "ogkm-610 (610.43.02)",
            option_env!("KAYFABE_VBIOS_ORACLE_610"),
        ),
    ]
    .into_iter()
    .filter_map(|(tag, p)| p.map(|p| (tag, p)))
    .collect()
}

/// Emit this test's gate line. Writes straight to `stderr` rather than through
/// `eprintln!`'s capture-aware path, so the **passing** arm is visible too — a gate whose
/// "it ran" marker only appears on failure cannot be counted, and counting it is the whole
/// non-vacuity argument.
fn report(test: &str, available: bool) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = if available {
        writeln!(err, "VBIOS-ORACLE-GATE: RAN {test}")
    } else {
        writeln!(
            err,
            "VBIOS-ORACLE-GATE: SKIPPED {test} — no vendored open-kernel-modules tree \
             (set KAYFABE_OGKM_580 / KAYFABE_OGKM_610). The test asserts NOTHING; this \
             line is the only record that it did not run."
        )
    };
}

/// `require_oracle!("name")` — gate the enclosing test on a built oracle, announcing both
/// arms. Returns the `(tag, path)` list.
macro_rules! require_oracle {
    ($name:expr) => {{
        let __o = oracles();
        report($name, !__o.is_empty());
        if __o.is_empty() {
            return;
        }
        __o
    }};
}

// ===========================================================================================
// Driving the oracle
// ===========================================================================================

/// One run of the real parser: the `key=value` fields, plus the driver's own diagnostics in
/// the order it emitted them.
struct Verdict {
    fields: BTreeMap<String, String>,
    logs: Vec<String>,
}

impl Verdict {
    fn get(&self, k: &str) -> Option<&str> {
        self.fields.get(k).map(String::as_str)
    }
    fn num(&self, k: &str) -> u64 {
        let v = self
            .get(k)
            .unwrap_or_else(|| panic!("oracle reported no `{k}`; it said:\n{self}"));
        let t = v.trim_start_matches("0x");
        if t.len() == v.len() {
            v.parse()
                .unwrap_or_else(|_| panic!("`{k}` = `{v}` is not a number"))
        } else {
            u64::from_str_radix(t, 16).unwrap_or_else(|_| panic!("`{k}` = `{v}` is not hex"))
        }
    }
    /// Every diagnostic joined, for substring assertions on the driver's exact wording.
    fn log_text(&self) -> String {
        self.logs.join("\n")
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (k, v) in &self.fields {
            writeln!(f, "  {k}={v}")?;
        }
        for l in &self.logs {
            writeln!(f, "  log: {l}")?;
        }
        Ok(())
    }
}

/// Run one oracle binary over `image`. `debug` drives `kgspIsDebugModeEnabled`.
fn ask(oracle: &str, image: &[u8], debug: bool) -> Verdict {
    let dir = std::env::temp_dir().join(format!(
        "kayfabe-vbios-oracle-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("image.rom");
    std::fs::write(&path, image).expect("write image");

    let out = Command::new(oracle)
        .arg(&path)
        .arg(if debug { "debug" } else { "prod" })
        .output()
        .unwrap_or_else(|e| panic!("could not run the oracle at {oracle}: {e}"));
    let _ = std::fs::remove_file(&path);

    assert!(
        out.status.success(),
        "the oracle process did not exit cleanly ({:?}) — the real parser CRASHED on this \
         image rather than returning a verdict, which is itself a finding.\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );

    let text = String::from_utf8_lossy(&out.stdout);
    let mut fields = BTreeMap::new();
    let mut logs = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("log=") {
            logs.push(rest.to_string());
        } else if let Some((k, v)) = line.split_once('=') {
            fields.insert(k.to_string(), v.to_string());
        }
    }
    Verdict { fields, logs }
}

/// The image every test starts from: the generator's own output for the one profile.
fn generated_image() -> Vec<u8> {
    let profile = profile_for_device_id(0x2504).expect("the GA106 profile row");
    build(profile, VbiosWire::Tu102Bit).expect("the generator accepts its own profile")
}

// ===========================================================================================
// Locating structures MECHANICALLY, so the mutants are not a second transcription
// ===========================================================================================
//
// Nothing below computes an offset from the C's layout rules. Each site is found by
// searching the produced image for the magic or pattern that identifies it, so a mistake
// here surfaces as "not found" rather than as a mutation applied to the wrong bytes.

fn find(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len())
        .position(|w| w == needle)
        .unwrap_or_else(|| panic!("could not locate {needle:x?} in the generated image"))
}

/// The falcon ucode payload: offset and length, found by its marker pattern.
fn payload(img: &[u8]) -> (usize, usize) {
    let pat: Vec<u8> = (0..64u32).map(|i| UCODE_MARKER ^ (i % 251) as u8).collect();
    let at = find(img, &pat);
    let mut n = 0usize;
    while at + n < img.len() && img[at + n] == UCODE_MARKER ^ (n % 251) as u8 {
        n += 1;
    }
    (at, n)
}

/// The V3 descriptor: found via its signature blob, which the generator fills with
/// `0,1,2,…`. The descriptor is exactly `FALCON_UCODE_DESC_V3_SIZE_44` bytes before it.
fn descriptor(img: &[u8]) -> usize {
    let sig: Vec<u8> = (0..32u8).collect();
    find(img, &sig) - FALCON_UCODE_DESC_V3_SIZE_44
}

/// The BIT header — `BIT_HEADER_ID` (0xB8FF, little-endian) followed by `"BIT\0"`.
fn bit_header(img: &[u8]) -> usize {
    find(img, b"\xff\xb8BIT\x00")
}

fn pcir(img: &[u8]) -> usize {
    find(img, b"PCIR")
}

fn npde(img: &[u8]) -> usize {
    find(img, b"NPDE")
}

// ===========================================================================================
// 1. The real parser accepts what we generate, and reports the profile back
// ===========================================================================================

#[test]
fn the_real_parser_accepts_our_generated_image() {
    let found = require_oracle!("the_real_parser_accepts_our_generated_image");
    let img = generated_image();
    let profile = profile_for_device_id(0x2504).unwrap();
    let fw = &profile.fwsec;

    for (tag, oracle) in found {
        let v = ask(oracle, &img, false);
        let ctx = format!("{tag}\n{v}");

        assert_eq!(
            v.num("extract_status"),
            0,
            "kgspExtractVbiosFromRom_TU102 refused:\n{ctx}"
        );
        assert_eq!(
            v.num("parse_status"),
            0,
            "kgspParseFwsecUcodeFromVbiosImg refused:\n{ctx}"
        );
        // ★ NOT redundant. `s_vbiosNewFlcnUcodeFromDesc` returns NV_OK with a NULL ucode
        // when the descriptor fill fails, so the pointer is the only signal that anything
        // was actually produced. (`ogkm-580: kernel_gsp_fwsec.c:1080-1094`.)
        assert_eq!(
            v.num("fwsec_present"),
            1,
            "FWSEC ucode is NULL despite NV_OK:\n{ctx}"
        );

        assert_eq!(v.num("bios_size"), img.len() as u64, "biosSize:\n{ctx}");
        assert_eq!(
            v.num("expansion_rom_offset"),
            0,
            "expansionRomOffset:\n{ctx}"
        );
        assert_eq!(
            v.num("vbios_version_combined"),
            (u64::from(profile.vbios_version) << 8) | u64::from(profile.vbios_oem_version),
            "the BIOSDATA token's version did not round-trip:\n{ctx}"
        );

        // Every geometry field the profile declares, read back out of the driver's own
        // KernelGspFlcnUcodeBootFromHs.
        let aligned = u64::from(fw.imem_load_size + fw.dmem_load_size)
            .div_ceil(u64::from(UCODE_ALIGN))
            * u64::from(UCODE_ALIGN);
        assert_eq!(v.num("ucode_size"), aligned, "ucode size:\n{ctx}");
        assert_eq!(
            v.num("imem_size"),
            u64::from(fw.imem_load_size),
            "IMEMLoadSize:\n{ctx}"
        );
        assert_eq!(
            v.num("imem_pa"),
            u64::from(fw.imem_phys_base),
            "IMEMPhysBase:\n{ctx}"
        );
        assert_eq!(
            v.num("imem_va"),
            u64::from(fw.imem_virt_base),
            "IMEMVirtBase:\n{ctx}"
        );
        assert_eq!(
            v.num("dmem_size"),
            u64::from(fw.dmem_load_size),
            "DMEMLoadSize:\n{ctx}"
        );
        assert_eq!(
            v.num("dmem_pa"),
            u64::from(fw.dmem_phys_base),
            "DMEMPhysBase:\n{ctx}"
        );
        // `dataOffset = imemSize` is the driver's own rule, not the descriptor's.
        assert_eq!(
            v.num("data_offset"),
            u64::from(fw.imem_load_size),
            "dataOffset:\n{ctx}"
        );
        assert_eq!(
            v.num("hs_sig_dmem_addr"),
            u64::from(fw.pkc_data_offset),
            "PKCDataOffset -> hsSigDmemAddr:\n{ctx}"
        );
        assert_eq!(v.num("ucode_id"), u64::from(fw.ucode_id), "UcodeId:\n{ctx}");
        assert_eq!(
            v.num("engine_id_mask"),
            u64::from(fw.engine_id_mask),
            "EngineIdMask:\n{ctx}"
        );
        assert_eq!(
            v.num("sig_count"),
            u64::from(fw.signature_count),
            "SignatureCount:\n{ctx}"
        );
        assert_eq!(
            v.num("sig_size"),
            BCRT30_RSA3K_SIG_SIZE as u64,
            "sigSize:\n{ctx}"
        );
        assert_eq!(
            v.num("signatures_total_size"),
            u64::from(fw.signature_count) * BCRT30_RSA3K_SIG_SIZE as u64,
            "signaturesTotalSize:\n{ctx}"
        );
        assert_eq!(
            v.num("vbios_sig_versions"),
            u64::from(fw.signature_versions),
            "SignatureVersions:\n{ctx}"
        );
        assert_eq!(
            v.num("interface_offset"),
            u64::from(fw.interface_offset),
            "InterfaceOffset:\n{ctx}"
        );

        // The bytes the driver would hand the falcon start with OUR marker pattern — so the
        // payload was not merely accepted, it was found where the builder put it.
        let head: String = (0..32u32)
            .map(|i| format!("{:02x}", UCODE_MARKER ^ (i % 251) as u8))
            .collect();
        assert_eq!(
            v.get("ucode_head"),
            Some(head.as_str()),
            "delivered ucode head:\n{ctx}"
        );

        // And the driver emitted no diagnostic at all.
        assert!(
            v.logs.is_empty(),
            "the driver complained about a clean image:\n{ctx}"
        );
    }
}

// ===========================================================================================
// 2. Both FWSEC arms — the generator's `fwsec_is_found_under_both_debug_and_prod`, checked
// ===========================================================================================

#[test]
fn fwsec_is_found_under_both_debug_and_prod_by_the_real_parser() {
    let found = require_oracle!("fwsec_is_found_under_both_debug_and_prod_by_the_real_parser");
    let img = generated_image();

    for (tag, oracle) in found {
        let prod = ask(oracle, &img, false);
        let dbg = ask(oracle, &img, true);
        for (mode, v) in [("prod", &prod), ("debug", &dbg)] {
            assert_eq!(v.num("parse_status"), 0, "{tag} {mode}:\n{v}");
            assert_eq!(v.num("fwsec_present"), 1, "{tag} {mode}:\n{v}");
        }
        // Stronger than "both found something": both arms select the SAME descriptor and
        // deliver the SAME bytes, which is what the three-appid layout claims.
        assert_eq!(
            prod.get("ucode_fnv1a"),
            dbg.get("ucode_fnv1a"),
            "{tag}: the debug and prod arms delivered different ucode\nprod:\n{prod}debug:\n{dbg}"
        );
        assert_eq!(
            prod.get("sig_fnv1a"),
            dbg.get("sig_fnv1a"),
            "{tag}: signature blobs differ"
        );
    }
}

// ===========================================================================================
// 3. The two vendored tags agree
// ===========================================================================================

#[test]
fn the_two_vendored_ogkm_tags_agree_on_our_image() {
    let found = require_oracle!("the_two_vendored_ogkm_tags_agree_on_our_image");
    if found.len() < 2 {
        // One tree present is not a disagreement; say so rather than passing quietly.
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "VBIOS-ORACLE-GATE: only {} tree(s) available — the 580/610 comparison did not run",
            found.len()
        );
        return;
    }
    let img = generated_image();
    for debug in [false, true] {
        let a = ask(found[0].1, &img, debug);
        let b = ask(found[1].1, &img, debug);
        assert_eq!(
            a.fields, b.fields,
            "{} and {} disagree (debug={debug}). The four files that define this path are \
             byte-identical at both tags (`kayfabe_abi::vbios::VbiosWire` records the diff), \
             so a disagreement here is a FINDING and the enum needs a second variant.\n\
             {}:\n{a}{}:\n{b}",
            found[0].0, found[1].0, found[0].0, found[1].0
        );
        assert_eq!(a.logs, b.logs, "the two tags emitted different diagnostics");
    }
}

// ===========================================================================================
// 4. ★ THE 16-BYTE SHIFT — the bite that failed to fire, put to the real parser
// ===========================================================================================

/// Move the ucode payload 16 bytes earlier in the image, exactly as the generator's own
/// docs describe the bite: *"with a zero payload, deliberately shifting this offset by 16
/// bytes left all 25 tests green"*.
fn shift_payload_left_16(img: &[u8]) -> Vec<u8> {
    let (at, n) = payload(img);
    assert!(
        at >= 16,
        "the payload is too close to the image start to shift"
    );
    let mut m = img.to_vec();
    let body: Vec<u8> = img[at..at + n].to_vec();
    m[at - 16..at - 16 + n].copy_from_slice(&body);
    for b in &mut m[at + n - 16..at + n] {
        *b = 0;
    }
    m
}

/// The same image with a **zero-filled** payload — the conditions under which the bite was
/// originally unobservable — optionally shifted 16 bytes left as well.
///
/// One function for both because the payload can only be LOCATED in the marker-filled
/// image: zero it first and there is nothing left to search for, which is the whole point.
fn zero_payload(img: &[u8], shift_left_16: bool) -> Vec<u8> {
    let (at, n) = payload(img);
    let mut m = img.to_vec();
    for b in &mut m[at..at + n] {
        *b = 0;
    }
    if shift_left_16 {
        assert!(
            at >= 16,
            "the payload is too close to the image start to shift"
        );
        for b in &mut m[at - 16..at - 16 + n] {
            *b = 0;
        }
    }
    m
}

#[test]
fn the_16_byte_payload_shift_is_not_a_verdict_the_real_parser_can_reach() {
    let found =
        require_oracle!("the_16_byte_payload_shift_is_not_a_verdict_the_real_parser_can_reach");
    let img = generated_image();
    let shifted = shift_payload_left_16(&img);
    assert_ne!(img, shifted, "the shift must actually change the image");

    for (tag, oracle) in found {
        let base = ask(oracle, &img, false);
        let moved = ask(oracle, &shifted, false);

        // ★ THE VERDICT, stated as an assertion so it cannot rot into prose: the real
        // driver ACCEPTS the shifted image. Placement is not something this parser checks —
        // `imageOffset = descOffset + descSize` is COMPUTED, never compared against where
        // the payload actually is, so a misplaced payload is in-bounds and therefore legal.
        assert_eq!(moved.num("extract_status"), 0, "{tag}:\n{moved}");
        assert_eq!(moved.num("parse_status"), 0, "{tag}:\n{moved}");
        assert_eq!(moved.num("fwsec_present"), 1, "{tag}:\n{moved}");
        assert!(
            moved.logs.is_empty(),
            "{tag}: unexpected diagnostics:\n{moved}"
        );

        // ★ And what DOES change: the bytes the driver would hand the falcon. That is the
        // whole value of the marker fill — it converts an unobservable placement into an
        // observable payload. With the marker, the shift is visible here:
        assert_ne!(
            base.get("ucode_fnv1a"),
            moved.get("ucode_fnv1a"),
            "{tag}: the shifted payload delivered IDENTICAL bytes — the marker fill has \
             stopped making placement observable, which is the exact condition under which \
             the original bite failed to fire.\nbase:\n{base}shifted:\n{moved}"
        );
    }
}

#[test]
fn with_a_zero_payload_the_shift_is_invisible_even_to_the_real_parser() {
    let found =
        require_oracle!("with_a_zero_payload_the_shift_is_invisible_even_to_the_real_parser");
    let img = generated_image();
    let zeroed = zero_payload(&img, false);
    let zeroed_and_shifted = zero_payload(&img, true);

    for (tag, oracle) in found {
        let a = ask(oracle, &zeroed, false);
        let b = ask(oracle, &zeroed_and_shifted, false);
        assert_eq!(a.num("parse_status"), 0, "{tag}:\n{a}");
        assert_eq!(b.num("parse_status"), 0, "{tag}:\n{b}");
        // ★ The bite's own conditions, reproduced against the REAL parser and the answer
        // is the same one the Rust suite gave: with a zero payload the shift is not merely
        // accepted, it is undetectable — the delivered bytes are IDENTICAL. So the original
        // green was not a transcription defect. The placement genuinely does not matter to
        // this parser, and the marker fill is what makes it matter to anything at all.
        assert_eq!(
            a.get("ucode_fnv1a"),
            b.get("ucode_fnv1a"),
            "{tag}: a zero payload's shift became observable — if this ever fires, the \
             finding recorded here is stale.\nunshifted:\n{a}shifted:\n{b}"
        );
        // The one thing that DOES move: the payload's new home overlaps the last 16 bytes
        // of the signature blob, so the blob the driver lifts out changes. Stated so the
        // result above is not read as "the shift changes nothing at all" — it changes
        // something the driver copies, just not the thing the shift was about.
        assert_ne!(
            a.get("sig_fnv1a"),
            b.get("sig_fnv1a"),
            "{tag}: the shifted payload did not disturb the signature blob either, which \
             means this mutation is not reaching the image at all.\n{a}\n{b}"
        );
    }
}

// ===========================================================================================
// 5. ★★ NON-VACUITY: the corruptions the real parser REFUSES, with its own words
// ===========================================================================================

/// One corruption: how to make it, and what the real driver must say about it.
struct Corruption {
    name: &'static str,
    mutate: fn(&mut Vec<u8>),
    /// `extract_status`, or `None` when extraction is expected to succeed.
    extract: u64,
    /// `parse_status`, or `None` when extraction already failed.
    parse: Option<u64>,
    /// A substring of a diagnostic the driver itself emits. Asserting the driver's exact
    /// wording is what makes this a measurement rather than a shrug.
    says: &'static str,
}

/// `NV_ERR_INVALID_DATA` (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h:66`).
const NV_ERR_INVALID_DATA: u64 = 0x0000_0025;
/// `NV_ERR_GENERIC` (`ogkm-580: :28`).
const NV_ERR_GENERIC: u64 = 0x0000_ffff;

const CORRUPTIONS: &[Corruption] = &[
    Corruption {
        name: "the PCI expansion-ROM signature (0xAA55 at offset 0)",
        mutate: |m| {
            m[0] = 0;
            m[1] = 0;
        },
        extract: NV_ERR_INVALID_DATA,
        parse: None,
        says: "did not find valid ROM signature",
    },
    Corruption {
        name: "the PCIR structure's signature",
        mutate: |m| {
            let at = pcir(m);
            m[at] = b'X';
        },
        extract: NV_ERR_INVALID_DATA,
        parse: None,
        says: "failed to locate expansion ROMs",
    },
    Corruption {
        name: "the NPDE sub-image length (a ROM claiming 32 MB)",
        mutate: |m| {
            let at = npde(m) + 8;
            m[at] = 0xff;
            m[at + 1] = 0xff;
        },
        extract: NV_ERR_INVALID_DATA,
        parse: None,
        says: "expansion ROM has exceedingly large size",
    },
    Corruption {
        name: "the BIT header's one-byte checksum",
        mutate: |m| {
            let at = bit_header(m) + 11;
            m[at] ^= 0xff;
        },
        extract: 0,
        parse: Some(NV_ERR_GENERIC),
        says: "failed to find BIT header in VBIOS image",
    },
    Corruption {
        name: "the FWSEC descriptor's declared size, one byte below the V3 minimum",
        mutate: |m| {
            let at = descriptor(m);
            let v = u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]]);
            let short = (v & 0xffff) | (((FALCON_UCODE_DESC_V3_SIZE_44 as u32) - 1) << 16);
            m[at..at + 4].copy_from_slice(&short.to_le_bytes());
        },
        extract: 0,
        parse: Some(NV_ERR_INVALID_DATA),
        says: "unexpected ucode desc version 0x3 or size 0x2b",
    },
    Corruption {
        name: "the FWSEC descriptor's version field",
        mutate: |m| {
            let at = descriptor(m);
            m[at + 1] = 5;
        },
        extract: 0,
        parse: Some(NV_ERR_INVALID_DATA),
        says: "unexpected ucode desc version 0x5",
    },
    Corruption {
        name: "the descriptor's version-available flag",
        mutate: |m| {
            let at = descriptor(m);
            m[at] &= !0x01;
        },
        extract: 0,
        parse: Some(NV_ERR_INVALID_DATA),
        says: "unexpected ucode desc version missing",
    },
    Corruption {
        name: "the falcon ucode table pointer, aimed out of range",
        mutate: |m| {
            // BIT token 1 is FALCON_DATA; its DataPtr addresses a u32 that addresses the
            // table. Both are found from the BIT header rather than computed.
            let tok = bit_header(m) + 12 + 8;
            let ptr = u32::from_le_bytes([m[tok + 4], m[tok + 5], m[tok + 6], m[tok + 7]]) as usize;
            m[ptr..ptr + 4].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        },
        extract: 0,
        parse: Some(NV_ERR_INVALID_DATA),
        says: "failed to read Falcon ucode header",
    },
    Corruption {
        name: "the FALCON_DATA token's data version",
        mutate: |m| {
            let at = bit_header(m) + 12 + 8 + 1;
            m[at] = 3;
        },
        extract: 0,
        parse: Some(NV_ERR_INVALID_DATA),
        says: "failed to parse FWSEC ucode desc from VBIOS image",
    },
];

#[test]
fn the_real_parser_rejects_corrupted_images() {
    let found = require_oracle!("the_real_parser_rejects_corrupted_images");
    let clean = generated_image();

    for (tag, oracle) in found {
        // The positive control, in the same test as the negatives: a harness that only
        // ever says "rejected" is as worthless as one that only ever says "accepted".
        let ok = ask(oracle, &clean, false);
        assert_eq!(
            ok.num("parse_status"),
            0,
            "{tag} rejected the clean image:\n{ok}"
        );

        for c in CORRUPTIONS {
            let mut m = clean.clone();
            (c.mutate)(&mut m);
            assert_ne!(m, clean, "{tag}: corruption `{}` changed nothing", c.name);

            let v = ask(oracle, &m, false);
            let ctx = format!("{tag} / corrupting {}\n{v}", c.name);

            assert_eq!(v.num("extract_status"), c.extract, "extract_status:\n{ctx}");
            match c.parse {
                None => assert!(
                    v.get("parse_status").is_none(),
                    "extraction was expected to fail before parsing:\n{ctx}"
                ),
                Some(want) => {
                    assert_eq!(v.num("parse_status"), want, "parse_status:\n{ctx}");
                    assert_eq!(
                        v.num("fwsec_present"),
                        0,
                        "a FWSEC ucode was produced from a corrupted image:\n{ctx}"
                    );
                }
            }
            assert!(
                v.log_text().contains(c.says),
                "the driver did not say `{}`. It said:\n{ctx}",
                c.says
            );
        }
    }
}

// ===========================================================================================
// 6. What the real parser does NOT check — measured, so nobody assumes otherwise
// ===========================================================================================

#[test]
fn the_real_parser_does_not_validate_the_descriptors_declared_size() {
    let found = require_oracle!("the_real_parser_does_not_validate_the_descriptors_declared_size");
    let clean = generated_image();

    for (tag, oracle) in found {
        for delta in [-1i32, 1] {
            let mut m = clean.clone();
            let at = descriptor(&m);
            let v0 = u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]]);
            let size = (v0 >> 16) & 0xffff;
            let bent = (v0 & 0xffff) | (size.wrapping_add(delta as u32) << 16);
            m[at..at + 4].copy_from_slice(&bent.to_le_bytes());

            let v = ask(oracle, &m, false);
            // ★ ACCEPTED, both directions. `descSize` is used as an ARITHMETIC INPUT —
            // `imageOffset = descOffset + descSize` and `signaturesTotalSize = descSize - 44`
            // (`ogkm-580: kernel_gsp_fwsec.c:937,985`) — and is only ever compared against
            // the V2/V3 minimum. So an off-by-one silently slides the payload origin AND
            // the signature-blob length. Nothing downstream of us would report it, which is
            // why the builder computing this value correctly is load-bearing and is not
            // something the driver will catch for us.
            assert_eq!(v.num("parse_status"), 0, "{tag} delta={delta}:\n{v}");
            assert_eq!(v.num("fwsec_present"), 1, "{tag} delta={delta}:\n{v}");
            assert_eq!(
                v.num("signatures_total_size"),
                (BCRT30_RSA3K_SIG_SIZE as i64 + i64::from(delta)) as u64,
                "{tag} delta={delta}: signaturesTotalSize tracks descSize verbatim:\n{v}"
            );
        }
    }
}

#[test]
fn the_pcir_image_length_is_overridden_by_the_npde() {
    let found = require_oracle!("the_pcir_image_length_is_overridden_by_the_npde");
    let clean = generated_image();

    for (tag, oracle) in found {
        let mut m = clean.clone();
        let at = pcir(&m) + 0x10; // OFFSETOF_PCI_DATA_STRUCT_IMAGE_LEN
        m[at] = 0xff;
        m[at + 1] = 0xff;

        let v = ask(oracle, &m, false);
        // ★ A ROM whose PCIR claims 32 MB is still ACCEPTED at its true size, because
        // `s_locateExpansionRoms` replaces `imgLen` with the NPDE's `subImgLen` whenever the
        // NPDE is present (`ogkm-580: kernel_gsp_vbios_tu102.c:351`). The 1 MB cap is
        // therefore enforced against the NPDE's number, not the PCIR's — which is why the
        // "exceedingly large size" corruption above has to bend the NPDE.
        assert_eq!(v.num("extract_status"), 0, "{tag}:\n{v}");
        assert_eq!(v.num("bios_size"), clean.len() as u64, "{tag}:\n{v}");
        assert_eq!(v.num("parse_status"), 0, "{tag}:\n{v}");
    }
}

// ===========================================================================================
// 7. The oracle is what it claims to be
// ===========================================================================================

#[test]
fn the_oracle_reads_the_image_through_the_prom_window() {
    let found = require_oracle!("the_oracle_reads_the_image_through_the_prom_window");
    let img = generated_image();
    for (tag, oracle) in found {
        let v = ask(oracle, &img, false);
        // The driver assembles the image itself out of 32-bit PROM reads; a harness that
        // handed it a buffer would report zero. The count is a floor, not an equality:
        // it is a property of the driver's read pattern, not of ours.
        let rd32 = v.num("prom_rd32");
        assert!(
            rd32 >= img.len() as u64 / 4,
            "{tag}: only {rd32} PROM dword reads for a {}-byte image — the image is not \
             being served through the register window:\n{v}",
            img.len()
        );
    }
}

// ===========================================================================================
// 8. ★★★ THE FWSEC INTERFACE WALK — the plane the parser cannot see
// ===========================================================================================
//
// `kgspParseFwsecUcodeFromVbiosImg` reads `InterfaceOffset` and copies it into
// `KernelGspFlcnUcodeBootFromHs` without ever following it, so every test above passes on an
// image whose DMEM payload is opaque noise. The stock 580.159.04 guest proved that the hard
// way: it got through this whole parser and then stopped one stage later, in
// `s_prepareForFwsec_TU102` → `s_vbiosPatchInterfaceData`, with
//
//     NVRM: s_vbiosPatchInterfaceData: failed to find required interface entry for FWSEC cmd 0x15
//     NVRM: s_prepareForFwsec_TU102: (note: VBIOS version 94.18.00.00.00)
//     NVRM: RmInitAdapter failed! (0x62:0x25:2028)
//
// The oracle now compiles that function too — see the include note in `oracle/vbios_oracle.c`
// — and runs it over the DMEM section the REAL parser produced, with `mappedDataSize` and
// `dataOffset` taken off the structure the real parser filled in.
//
// ★★ WHAT THIS STILL CANNOT SEE, and it is the same shape as the payload-shift finding
// above: the walk is the LAST reader before the falcon. It checks that the table is
// reachable and in bounds; it does not check that the mapper's `cmd_out_buffer_*` are
// sensible, that `signature`/`version`/`ucode_feature` mean anything, or that the falcon
// could do something with the command once it is delivered — because in Mode 2 the falcon is
// us, and nothing in NVIDIA's tree models it. A green here says the driver will HAND US the
// command. It says nothing about what happens next.

/// `NV_ERR_INVALID_OFFSET` (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h:84`).
const NV_ERR_INVALID_OFFSET: u64 = 0x0000_0037;

/// The interface table's location in the image, found by the header's own byte pattern
/// followed by a `_DMEMMAPPER` entry — not computed from the profile's offsets, so a
/// builder that wrote it in the wrong place surfaces as "not found" or "found twice".
fn interface_table(img: &[u8]) -> usize {
    let hdr = [
        1u8,
        FalconApplicationInterfaceHeaderV1::SIZE as u8,
        FalconApplicationInterfaceEntryV1::SIZE as u8,
        INTERFACE_ENTRY_COUNT,
    ];
    let mut id = [0u8; 4];
    id.copy_from_slice(&FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER.to_le_bytes());
    let mut needle = hdr.to_vec();
    needle.extend_from_slice(&id);
    let hits: Vec<usize> = img
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle.as_slice())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "the interface header must appear exactly once in the image, found {}",
        hits.len()
    );
    hits[0]
}

/// The mapper as the driver would find it, decoded out of a DMEM hex dump the oracle
/// produced.
fn mapper_from(dmem: &[u8], at: u32) -> FalconApplicationInterfaceDmemMapperV3 {
    FalconApplicationInterfaceDmemMapperV3::decode(&dmem[at as usize..])
        .expect("the mapper must decode out of DMEM")
}

fn hex_field(v: &Verdict, k: &str) -> Vec<u8> {
    let s = v
        .get(k)
        .unwrap_or_else(|| panic!("the oracle reported no `{k}`; it said:\n{v}"));
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn the_real_interface_walk_delivers_both_fwsec_commands() {
    let found = require_oracle!("the_real_interface_walk_delivers_both_fwsec_commands");
    let img = generated_image();
    let fw = &profile_for_device_id(0x2504).unwrap().fwsec;

    for (tag, oracle) in found {
        let v = ask(oracle, &img, false);
        let ctx = format!("{tag}\n{v}");

        // ★ THE ROW. Both commands walk the table clean, with no diagnostic at all — the
        // 0x15 in the guest's failure message is the FRTS one.
        assert_eq!(v.num("walk_frts_status"), 0, "FRTS interface walk:\n{ctx}");
        assert_eq!(v.num("walk_sb_status"), 0, "SB interface walk:\n{ctx}");
        assert!(
            v.logs.is_empty(),
            "the driver complained while walking a clean table:\n{ctx}"
        );

        let before = hex_field(&v, "walk_dmem_before");
        let after_frts = hex_field(&v, "walk_frts_dmem");
        let after_sb = hex_field(&v, "walk_sb_dmem");
        assert_eq!(
            before.len(),
            fw.dmem_load_size as usize,
            "DMEM size:\n{ctx}"
        );

        // The mapper the walk found is the one the profile declared, and `init_cmd` moved
        // from the 0 the builder emits to the command the driver issues. Emitting 0 is what
        // makes that write observable at all.
        assert_eq!(mapper_from(&before, fw.dmem_mapper_offset).init_cmd, 0);
        assert_eq!(
            mapper_from(&after_frts, fw.dmem_mapper_offset).init_cmd,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_FRTS,
            "the driver did not write the FRTS command into our mapper:\n{ctx}"
        );
        assert_eq!(
            mapper_from(&after_sb, fw.dmem_mapper_offset).init_cmd,
            FALCON_APPLICATION_INTERFACE_DMEM_MAPPER_V3_CMD_SB,
            "the driver did not write the SB command into our mapper:\n{ctx}"
        );

        // …and each command payload landed at `cmd_in_buffer_offset`, byte for byte,
        // compared against the bytes the harness actually passed rather than a constant.
        for (what, dumped, cmd_key) in [
            ("FRTS", &after_frts, "frts_cmd_bytes"),
            ("SB", &after_sb, "sb_cmd_bytes"),
        ] {
            let cmd = hex_field(&v, cmd_key);
            let at = fw.cmd_in_buffer_offset as usize;
            assert_eq!(
                &dumped[at..at + cmd.len()],
                &cmd[..],
                "{what}: the command did not land at cmd_in_buffer_offset:\n{ctx}"
            );
            assert_ne!(
                &before[at..at + cmd.len()],
                &cmd[..],
                "{what}: the command bytes were already there, so the copy is unobservable \
                 — the marker-fill lesson:\n{ctx}"
            );
        }

        // The table is where the descriptor says it is. ★ Unlike the ucode payload, whose
        // placement the parser CANNOT check (test 4 above), this one it can and does: the
        // walk starts at `dataOffset + interfaceOffset` and finds nothing if the builder
        // wrote the table elsewhere.
        let p = ask(oracle, &img, false);
        let payload_at = interface_table(&img);
        assert_eq!(
            p.num("interface_offset"),
            u64::from(fw.interface_offset),
            "InterfaceOffset:\n{ctx}"
        );
        assert!(payload_at > 0, "the table must not be at image offset 0");
    }
}

/// The sizes the builder computes from the generated mirrors equal `sizeof()` as the C
/// compiler lays the same typedefs out — including
/// `sizeof(FWSECLIC_FRTS_CMD)`, which this repository does NOT mirror and which
/// [`FWSEC_CMD_BUFFER_MIN`] only bounds from below.
#[test]
fn the_declared_command_buffer_holds_the_real_fwsec_command() {
    let found = require_oracle!("the_declared_command_buffer_holds_the_real_fwsec_command");
    let img = generated_image();
    let fw = &profile_for_device_id(0x2504).unwrap().fwsec;

    for (tag, oracle) in found {
        let v = ask(oracle, &img, false);
        let ctx = format!("{tag}\n{v}");

        assert_eq!(
            v.num("interface_hdr_size"),
            FalconApplicationInterfaceHeaderV1::SIZE as u64,
            "interface header:\n{ctx}"
        );
        assert_eq!(
            v.num("interface_entry_size"),
            FalconApplicationInterfaceEntryV1::SIZE as u64,
            "interface entry:\n{ctx}"
        );
        assert_eq!(
            v.num("dmem_mapper_size"),
            FalconApplicationInterfaceDmemMapperV3::SIZE as u64,
            "DMEM mapper:\n{ctx}"
        );
        assert_eq!(
            v.num("dmemmapper_entry_id"),
            u64::from(FALCON_APPLICATION_INTERFACE_ENTRY_ID_DMEMMAPPER),
            "the `_DMEMMAPPER` id:\n{ctx}"
        );

        // ★ The bound the builder could only state from below. `FWSEC_CMD_BUFFER_MIN` is
        // `sizeof(READ_VBIOS_DESC) + sizeof(FRTS_REGION_DESC)`; the compiler is free to pad
        // the aggregate's tail beyond that, and here it does not — but the profile's
        // declared buffer is checked against the COMPILED size either way.
        let frts = v.num("frts_cmd_size");
        assert!(
            frts >= u64::from(FWSEC_CMD_BUFFER_MIN),
            "sizeof(FWSECLIC_FRTS_CMD) is {frts}, below the sum of its members:\n{ctx}"
        );
        assert!(
            u64::from(fw.cmd_in_buffer_size) >= frts,
            "cmd_in_buffer_size is {} but the driver copies {frts} bytes into it:\n{ctx}",
            fw.cmd_in_buffer_size
        );
        assert!(
            u64::from(fw.cmd_in_buffer_size) >= v.num("read_vbios_desc_size"),
            "cmd_in_buffer_size does not hold the SB command either:\n{ctx}"
        );
    }
}

// ===========================================================================================
// 9. ★★ NON-VACUITY on the interface plane: the tables the real walk REFUSES
// ===========================================================================================

/// One DMEM corruption, applied to the image before the parser ever sees it.
struct DmemCorruption {
    name: &'static str,
    /// `(image, table offset within the image)`.
    mutate: fn(&mut [u8], usize),
    /// What `s_vbiosPatchInterfaceData` must return for the FRTS command.
    walk: u64,
    /// A substring of the driver's own diagnostic, or `""` when it emits none.
    says: &'static str,
}

const DMEM_CORRUPTIONS: &[DmemCorruption] = &[
    DmemCorruption {
        name: "the interface header's entryCount, dropped to 1",
        mutate: |m, at| m[at + 3] = 1,
        walk: NV_ERR_INVALID_DATA,
        says: "too few interface entires found for FWSEC cmd 0x15",
    },
    DmemCorruption {
        name: "both entries' id, so no _DMEMMAPPER remains",
        mutate: |m, at| {
            let e = at + FalconApplicationInterfaceHeaderV1::SIZE;
            m[e] = 0xEE;
            m[e + FalconApplicationInterfaceEntryV1::SIZE] = 0xEE;
        },
        // ★ THE EXACT FAILURE THE STOCK GUEST REPORTED, reproduced on demand.
        walk: NV_ERR_INVALID_DATA,
        says: "failed to find required interface entry for FWSEC cmd 0x15",
    },
    DmemCorruption {
        name: "both entries' dmemOffset, aimed past the end of DMEM",
        mutate: |m, at| {
            let e = at + FalconApplicationInterfaceHeaderV1::SIZE;
            for i in 0..usize::from(INTERFACE_ENTRY_COUNT) {
                let o = e + i * FalconApplicationInterfaceEntryV1::SIZE + 4;
                m[o..o + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
            }
        },
        walk: NV_ERR_INVALID_OFFSET,
        says: "",
    },
];

#[test]
fn the_real_interface_walk_refuses_corrupted_tables() {
    let found = require_oracle!("the_real_interface_walk_refuses_corrupted_tables");
    let clean = generated_image();
    let table_at = interface_table(&clean);

    for (tag, oracle) in found {
        // The positive control lives in the same test as the negatives.
        let ok = ask(oracle, &clean, false);
        assert_eq!(
            ok.num("walk_frts_status"),
            0,
            "{tag} refused the clean table:\n{ok}"
        );

        for c in DMEM_CORRUPTIONS {
            let mut m = clean.clone();
            (c.mutate)(&mut m, table_at);
            assert_ne!(m, clean, "{tag}: corruption `{}` changed nothing", c.name);

            let v = ask(oracle, &m, false);
            let ctx = format!("{tag} / corrupting {}\n{v}", c.name);

            // The ROM still parses — this plane is downstream of everything above, which is
            // exactly why the parser tests could not see it.
            assert_eq!(v.num("parse_status"), 0, "the ROM must still parse:\n{ctx}");
            assert_eq!(
                v.num("fwsec_present"),
                1,
                "FWSEC must still be found:\n{ctx}"
            );
            assert_eq!(v.num("walk_frts_status"), c.walk, "walk status:\n{ctx}");
            if !c.says.is_empty() {
                assert!(
                    v.log_text().contains(c.says),
                    "the driver did not say `{}`. It said:\n{ctx}",
                    c.says
                );
            }
        }
    }
}

#[test]
fn the_oracle_binaries_exist_where_the_build_script_said() {
    let found = require_oracle!("the_oracle_binaries_exist_where_the_build_script_said");
    for (tag, oracle) in found {
        assert!(
            Path::new(oracle).is_file(),
            "{tag}: the build script recorded {oracle}, which is not a file"
        );
    }
}
