//! ★ 2026-10-14 — **the VIDEO tier of the T-mode rewriter** (Win11 Edge/YouTube TDR, defect 2 of
//! `traces/win11_userring_20261014/`).
//!
//! A guest-KERNEL channel on a video engine (NVDEC, NVENC, NVJPG, OFA) is born Translated on a host
//! ring of that engine, holding a real owned object of the family's video class
//! (`crate::host::HostRing::video_context`). Its pushbuffer was decoded against the COPY-ENGINE
//! table — subchannel 4 is the CE subchannel on the graphics runlist — so the first video method
//! the Windows kernel driver sends (`SEMAPHORE_A`, `0x240`, measured: run 2990 `0c5fedff`, kernel
//! NVDEC channel `0xc1d00018:0xff040003`, `NoCeObject { subch: 4, method: 576 }`) killed the channel
//! and the guest TDR'd. On a video runlist there is no copy engine: subchannels 0-4 reach the
//! channel's one video engine (public `dev_ram.ref`: hardware subchannels 0-4, software 5-7).
//!
//! The method SPACE is the class's own, DERIVED by `tools/derive_video_methods.py` from NVIDIA's
//! public class headers (open-gpu-doc `classes/video/`; ogkm carries only the class ids) into
//! [`crate::video_generated`]: every method offset and name, and the fields and named values of
//! the semaphore methods. What is maintained by hand here is only the DISPOSITION, keyed by the
//! method's NAME (so no offset is written by hand):
//!
//! - `NOP` — address-free, consumed (as the CE tier's `NOP`);
//! - `SEMAPHORE_A` / `_B` — an ADDRESS (held; at `SEMAPHORE_D` resolved through the channel's
//!   placement rows and emitted by the perimeter as a window address, as every host semaphore);
//! - `SEMAPHORE_C`, `SET_SEMAPHORE_PAYLOAD_LOWER` / `_UPPER` — payload data (held, re-emitted);
//! - `SEMAPHORE_D` — the trigger, decoded against the class's OWN fields: every set bit inside a
//!   named field, every field value a named value, the operation `RELEASE` or `TRAP` (a release /
//!   an engine non-stall trap, `NV*B0_OS_INTERRUPT_TRAP_NONSTALL`);
//! - every other method the header names — `EXECUTE` (codec work), the `*_OFFSET` operands,
//!   `CTX_SAVE_AREA` / `CTX_SWITCH`, `SET_MONITORED_FENCE_SIGNAL_ADDRESS_BASE_*` (an address no
//!   rewriter authors), `PM_TRIGGER*` — is REFUSED BY NAME: the T-mode space has no row for a
//!   codec operand (a decode surface is not a semaphore), and none is forwarded;
//! - an offset the header does not name is unclassified (refused).

/// One field of a video method, as the public header defines it.
#[derive(Debug, PartialEq, Eq)]
pub struct VideoField {
    /// The method the field belongs to (its name without the class prefix).
    pub method: &'static str,
    /// The field's name.
    pub name: &'static str,
    /// Its high bit.
    pub hi: u8,
    /// Its low bit.
    pub lo: u8,
    /// Its named values (empty: any value).
    pub values: &'static [(&'static str, u32)],
}

impl VideoField {
    /// The field's mask in the method word.
    #[must_use]
    pub const fn mask(&self) -> u32 {
        let width = self.hi - self.lo + 1;
        let ones = if width >= 32 {
            u32::MAX
        } else {
            (1u32 << width) - 1
        };
        ones << self.lo
    }

    /// The field's value in `word`.
    #[must_use]
    pub const fn get(&self, word: u32) -> u32 {
        (word & self.mask()) >> self.lo
    }

    /// The name of `value`, if the header names it.
    #[must_use]
    pub fn value_name(&self, value: u32) -> Option<&'static str> {
        self.values
            .iter()
            .find(|(_, v)| *v == value)
            .map(|(n, _)| *n)
    }
}

/// One video class: every method its public header names, and the semaphore methods' fields.
#[derive(Debug, PartialEq, Eq)]
pub struct VideoClass {
    /// The class id.
    pub class: u32,
    /// `(offset, name)` of every method.
    pub methods: &'static [(u32, &'static str)],
    /// The fields of the semaphore methods.
    pub fields: &'static [VideoField],
}

impl VideoClass {
    /// The derived table of `class`, or `None` (no public header: the tier refuses by name).
    #[must_use]
    pub fn of_class(class: u32) -> Option<&'static VideoClass> {
        crate::video_generated::CLASSES
            .iter()
            .find(|c| c.class == class)
    }

    /// The name of method `m`.
    #[must_use]
    pub fn method_name(&self, m: u32) -> Option<&'static str> {
        self.methods.iter().find(|(o, _)| *o == m).map(|(_, n)| *n)
    }

    /// The offset of the method named `name`.
    #[must_use]
    pub fn offset(&self, name: &str) -> Option<u32> {
        self.methods
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(o, _)| *o)
    }

    /// The field `name` of `method`.
    #[must_use]
    pub fn field(&self, method: &str, name: &str) -> Option<&'static VideoField> {
        self.fields
            .iter()
            .find(|f| f.method == method && f.name == name)
    }

    fn fields_of<'a>(&'a self, method: &'a str) -> impl Iterator<Item = &'static VideoField> + 'a {
        self.fields.iter().filter(move |f| f.method == method)
    }

    /// The bits of `SEMAPHORE_A`'s `UPPER` field: the semaphore address is `32 + bits` wide.
    #[must_use]
    pub fn upper_bits(&self) -> Option<u32> {
        let f = self.field("SEMAPHORE_A", "UPPER")?;
        (f.lo == 0).then_some(u32::from(f.hi) + 1)
    }
}

/// A video method's disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMethod {
    /// `NOP` — consumed.
    Nop,
    /// `SEMAPHORE_A` — the address's upper bits.
    SemA,
    /// `SEMAPHORE_B` — the address's lower 32 bits.
    SemB,
    /// `SEMAPHORE_C` — the 32-bit payload.
    SemC,
    /// `SET_SEMAPHORE_PAYLOAD_LOWER`.
    PayloadLower,
    /// `SET_SEMAPHORE_PAYLOAD_UPPER`.
    PayloadUpper,
    /// `SEMAPHORE_D` — the trigger.
    SemD,
    /// Named by the header, refused by name.
    Refused(&'static str),
}

/// The disposition of method `m` on `class`: `None` = the header names no such method.
#[must_use]
pub fn video_method(class: &VideoClass, m: u32) -> Option<VideoMethod> {
    let name = class.method_name(m)?;
    Some(match name {
        "NOP" => VideoMethod::Nop,
        "SEMAPHORE_A" => VideoMethod::SemA,
        "SEMAPHORE_B" => VideoMethod::SemB,
        "SEMAPHORE_C" => VideoMethod::SemC,
        "SET_SEMAPHORE_PAYLOAD_LOWER" => VideoMethod::PayloadLower,
        "SET_SEMAPHORE_PAYLOAD_UPPER" => VideoMethod::PayloadUpper,
        "SEMAPHORE_D" => VideoMethod::SemD,
        other => VideoMethod::Refused(other),
    })
}

/// A `SEMAPHORE_D` word that passed the class's own fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSemD {
    /// The word, re-emitted as decoded (every bit in a named field, every value named).
    pub word: u32,
    /// The bytes the operation may write at the semaphore address: 4 (`STRUCTURE_SIZE_ONE`, 32-bit
    /// payload), 8 (`_TWO`, or a 64-bit payload), 16 (`_FOUR`: payload and timestamp).
    pub bytes: u64,
    /// `PAYLOAD_SIZE_64BIT`: the payload registers carry it.
    pub payload64: bool,
}

/// Decode `word` as `class`'s `SEMAPHORE_D`.
///
/// # Errors
/// What is wrong with it, by name: a bit outside every named field, a field value the header
/// does not name, or an operation other than `RELEASE` / `TRAP`.
pub fn decode_semaphore_d(class: &VideoClass, word: u32) -> Result<VideoSemD, &'static str> {
    let mut named = 0u32;
    for f in class.fields_of("SEMAPHORE_D") {
        named |= f.mask();
        if !f.values.is_empty() && f.value_name(f.get(word)).is_none() {
            return Err("SEMAPHORE_D field value the class header does not name");
        }
    }
    if word & !named != 0 {
        return Err("SEMAPHORE_D beyond its named fields");
    }
    let named_value = |field: &str| {
        class
            .field("SEMAPHORE_D", field)
            .and_then(|f| f.value_name(f.get(word)))
    };
    match named_value("OPERATION") {
        Some("RELEASE" | "TRAP") => {}
        _ => return Err("SEMAPHORE_D operation other than RELEASE or TRAP"),
    }
    let payload64 = named_value("PAYLOAD_SIZE") == Some("64BIT");
    let mut bytes = match named_value("STRUCTURE_SIZE") {
        Some("ONE") => 4,
        Some("TWO") => 8,
        Some("FOUR") => 16,
        _ => return Err("SEMAPHORE_D structure size the tier does not size"),
    };
    if payload64 {
        bytes = bytes.max(8);
    }
    Ok(VideoSemD {
        word,
        bytes,
        payload64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_derived_video_class_has_the_semaphore_rows_the_tier_needs() {
        for c in crate::video_generated::CLASSES {
            for m in [
                "NOP",
                "SEMAPHORE_A",
                "SEMAPHORE_B",
                "SEMAPHORE_C",
                "SEMAPHORE_D",
            ] {
                assert!(c.offset(m).is_some(), "{:#x} lacks {m}", c.class);
            }
            assert!(c.field("SEMAPHORE_D", "OPERATION").is_some());
            assert!(c.field("SEMAPHORE_D", "STRUCTURE_SIZE").is_some());
            assert!(c.upper_bits().is_some_and(|b| (8..=25).contains(&b)));
            // offsets are method offsets
            assert!(c.methods.iter().all(|(o, _)| o % 4 == 0 && *o < 0x4000));
        }
    }

    #[test]
    fn the_ada_decoder_is_derived_with_its_measured_semaphore_layout() {
        let c = VideoClass::of_class(0xC9B0).expect("NVC9B0 derived");
        // the measured refusal: subchannel 4, method 0x240 on the kernel NVDEC channel
        assert_eq!(video_method(c, 0x240), Some(VideoMethod::SemA));
        assert_eq!(video_method(c, 0x244), Some(VideoMethod::SemB));
        assert_eq!(video_method(c, 0x248), Some(VideoMethod::SemC));
        assert_eq!(video_method(c, 0x304), Some(VideoMethod::SemD));
        assert_eq!(video_method(c, 0x254), Some(VideoMethod::PayloadLower));
        assert_eq!(
            video_method(c, 0x300),
            Some(VideoMethod::Refused("EXECUTE"))
        );
        assert_eq!(
            video_method(c, 0x25C),
            Some(VideoMethod::Refused(
                "SET_MONITORED_FENCE_SIGNAL_ADDRESS_BASE_A"
            ))
        );
        assert_eq!(
            video_method(c, 0x404),
            Some(VideoMethod::Refused("SET_DRV_PIC_SETUP_OFFSET"))
        );
        assert_eq!(
            video_method(c, 0x104),
            None,
            "an offset the header does not name"
        );
        assert_eq!(c.upper_bits(), Some(8));
        // the two words the Windows kernel driver sent: a 32-bit release, then a non-stall trap
        assert_eq!(
            decode_semaphore_d(c, 0),
            Ok(VideoSemD {
                word: 0,
                bytes: 4,
                payload64: false
            })
        );
        assert_eq!(decode_semaphore_d(c, 0x3_0000).map(|d| d.bytes), Ok(4));
        // FOUR = payload + timestamp; 64-bit payload
        assert_eq!(decode_semaphore_d(c, 1).map(|d| d.bytes), Ok(16));
        assert_eq!(
            decode_semaphore_d(c, 1 << 24).map(|d| (d.bytes, d.payload64)),
            Ok((8, true))
        );
        // RESERVED operations, an unnamed STRUCTURE_SIZE (3), unnamed TRAP_TYPE (3), stray bits
        assert!(decode_semaphore_d(c, 1 << 16).is_err());
        assert!(decode_semaphore_d(c, 2 << 16).is_err());
        assert!(decode_semaphore_d(c, 3).is_err());
        assert!(decode_semaphore_d(c, 3 << 22 | 3 << 16).is_err());
        assert!(decode_semaphore_d(c, 1 << 4).is_err());
        assert!(decode_semaphore_d(c, 1 << 31).is_err());
    }

    #[test]
    fn a_field_layout_is_per_class_never_assumed() {
        // NVC9B7's PAYLOAD_SIZE is bit 2 (not 24), NVC5B0's STRUCTURE_SIZE is one bit, NVC9D1's
        // SEMAPHORE_A UPPER is 25 bits: the same word means different things per class.
        let enc = VideoClass::of_class(0xC9B7).unwrap();
        assert_eq!(
            enc.field("SEMAPHORE_D", "PAYLOAD_SIZE")
                .map(|f| (f.hi, f.lo)),
            Some((2, 2))
        );
        let tu = VideoClass::of_class(0xC5B0).unwrap();
        assert!(
            decode_semaphore_d(tu, 2).is_err(),
            "STRUCTURE_SIZE is 0:0 on C5B0"
        );
        assert!(
            decode_semaphore_d(tu, 1 << 24).is_err(),
            "C5B0 names no PAYLOAD_SIZE"
        );
        let jpg = VideoClass::of_class(0xC9D1).unwrap();
        assert_eq!(jpg.upper_bits(), Some(25));
        // NVC7B7 names MONITORED_FENCE operations: refused by name like RESERVED
        let c7b7 = VideoClass::of_class(0xC7B7).unwrap();
        assert!(decode_semaphore_d(c7b7, 1 << 16).is_err());
        assert!(decode_semaphore_d(c7b7, 3 << 16).is_ok());
    }
}
