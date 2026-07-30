//! The hand-written substrate the generated code sits on: the error type, the
//! layout-description types, and the four little-endian readers.
//!
//! Everything here is safe Rust. There is no `transmute`, no pointer cast, no
//! `bytemuck`: a wire struct is decoded by **reading its fields out of a byte
//! slice**, which is why `#![forbid(unsafe_code)]` holds across a crate whose
//! entire job is `#[repr(C)]` layouts. The `#[repr(C)]` mirrors exist to be a
//! layout *oracle* (`size_of` / `offset_of!` against the generator's own
//! computation) and to be the decoded value — never a cast target.
//!
//! # This crate parses untrusted input
//!
//! Every byte a decoder here sees came from the guest. So the rules are:
//!
//! 1. **Short is refused, loudly.** Never zero-extend a truncated struct —
//!    that is `abi_struct_truncation` verbatim, and it fails *silently*, which
//!    is the worst possible shape for an ABI bug.
//! 2. **No panics on any input.** Not `unwrap`, not indexing, not arithmetic
//!    that can overflow. A guest-triggerable panic in the VMM is a denial of
//!    service against every other guest on the box.
//! 3. **Writeback touches only named fields.** See `encode_into` on any
//!    generated struct.

/// Everything a decoder can refuse to do.
///
/// Deliberately **not** a catch-all: each variant names the exact failure and
/// carries the numbers a reader needs, because `testing_doctrine.md` §2 forbids
/// asserting on `is_err()` and a single opaque variant would force it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AbiError {
    /// The buffer is shorter than the struct the ABI table says is there.
    ///
    /// This is the load-bearing one. A guest that under-sends is either buggy or
    /// probing; either way the answer is a refusal naming the struct, never a
    /// best-effort partial decode.
    Truncated {
        /// The C typedef name, e.g. `"NVOS46_PARAMETERS"`.
        c_name: &'static str,
        /// Bytes the layout requires.
        need: usize,
        /// Bytes actually available.
        got: usize,
    },
    /// A field read fell outside the buffer. Unreachable via the generated
    /// `decode`/`encode_into` (they length-check first) but kept as a real
    /// variant so the primitive readers are total functions rather than
    /// panicking ones.
    OutOfRange {
        /// Byte offset the read started at.
        offset: usize,
        /// Width of the read.
        width: usize,
        /// Length of the buffer.
        len: usize,
    },
    /// No ABI table is registered for the requested driver version, because it
    /// predates the oldest version this build knows how to decode.
    ///
    /// A **loud miss**, never a fallback to the nearest table: the C artifact's
    /// `nvkvm_abi_by_id` returns the 570 profile for anything unrecognised
    /// (`src/common/nvkvm_abi.h:105-110`), which means an unknown driver silently
    /// gets 575's struct sizes.
    NoTableForVersion {
        /// Major version requested.
        major: u16,
        /// Minor version requested.
        minor: u16,
        /// Patch version requested.
        patch: u16,
    },
    /// An `NV_ESC_RM_ALLOC` arrived with an ioctl size matching neither the v1
    /// (`NVOS21`, 32 bytes) nor the v2 (`NVOS64`, 48 bytes) shape.
    ///
    /// Refused rather than guessed from `bytes.len()`: choosing the wider shape
    /// for a narrower struct reads `pRightsRequested` out of `paramsSize` and
    /// `status`, which is the `nvos64_abi_fix` incident's shape exactly.
    UnknownAllocWire {
        /// The size the ioctl declared.
        ioctl_size: usize,
    },
    /// A GSP RPC envelope declared a `length` that is not physically possible:
    /// smaller than the envelope itself, or larger than the buffer it arrived
    /// in. Guest-controlled, so it is checked before it is used for anything.
    RpcLength {
        /// The declared `length` field.
        declared: u32,
        /// Bytes actually available.
        available: usize,
    },
    /// ★★★ **A `PROMOTE_CTX` declared more entries than `promoteEntry[]` has.**
    ///
    /// `entryCount` is a guest-supplied count into a **fixed** 16-element array. The C
    /// artifact clamped it to `64` — with a comment claiming the constant is `20` — and
    /// then read `params+560 … params+2096`, i.e. 1536 bytes past the struct, out of the
    /// guest-writable 4096-byte queue element. Every 32-byte window there with a plausible
    /// `va`/`size` went into its address table under the guest's own choice of client, on
    /// the hot resolution path. Three numbers, all different, none of them 16.
    ///
    /// Refused by name rather than clamped: a clamp turns "the guest declared nonsense"
    /// into "we decoded a prefix of the nonsense", which is `abi_struct_truncation`
    /// pointed the other way.
    PromoteEntryCount {
        /// `entryCount`, as the guest declared it.
        declared: u32,
        /// `NV2080_CTRL_GPU_PROMOTE_CONTEXT_MAX_ENTRIES` — 16 at 580.159.04 and 610.43.02.
        max: usize,
    },
    /// A `PROMOTE_CTX` entry whose `physAttr[1:0]` is `3` — a value NVIDIA has not
    /// defined (`_VIDMEM` = 0, `_COH_SYS` = 1, `_NCOH_SYS` = 2).
    ///
    /// Refused rather than folded into sysmem. The C artifact's
    /// `(physAttr & 0x3u) != 0` maps `3` to sysmem *and* collapses the coherent /
    /// non-coherent distinction that `kayfabe_arch::Aperture` models, so porting it would
    /// be a strict regression against this port's own vocabulary.
    PromoteAperture {
        /// Index of the offending entry within `promoteEntry[]`.
        entry: usize,
        /// The whole `physAttr` word, so a reader can see the other flags too.
        phys_attr: u32,
    },
    /// A `PROMOTE_CTX` that used the **legacy** `hVirtMemory` / `(virtAddress, size)`
    /// path instead of `promoteEntry[]`.
    ///
    /// Both real producers leave all three zero and set `entryCount > 0`
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:131`,
    /// `.../kernel_graphics_context.c:2166`). The legacy shape is refused rather than
    /// guessed at: it declares one mapping through a *memory handle* this port does not
    /// resolve, and inventing a reading for it would be a decoder with no oracle.
    PromoteLegacyShape {
        /// `hVirtMemory` @ +20.
        h_virt_memory: u32,
        /// `virtAddress` @ +24.
        virt_address: u64,
        /// `size` @ +32.
        size: u64,
    },
    /// A GSP RPC envelope carried the wrong `signature`. Not a security control
    /// on its own — a guest can write the right bytes — but a decode that
    /// ignores it would happily parse a buffer that was never an RPC at all.
    RpcSignature {
        /// The signature found.
        found: u32,
        /// The signature required.
        expected: u32,
    },
}

impl core::fmt::Display for AbiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { c_name, need, got } => {
                write!(f, "{c_name}: need {need} bytes, got {got}")
            }
            Self::OutOfRange { offset, width, len } => {
                write!(
                    f,
                    "read of {width} bytes at +{offset} exceeds the {len}-byte buffer"
                )
            }
            Self::NoTableForVersion {
                major,
                minor,
                patch,
            } => {
                write!(f, "no ABI table for driver {major}.{minor}.{patch}")
            }
            Self::UnknownAllocWire { ioctl_size } => {
                write!(
                    f,
                    "RM_ALLOC ioctl size {ioctl_size} is neither NVOS21 (32) nor NVOS64 (48)"
                )
            }
            Self::RpcLength {
                declared,
                available,
            } => {
                write!(
                    f,
                    "RPC declared length {declared} against {available} available bytes"
                )
            }
            Self::RpcSignature { found, expected } => {
                write!(f, "RPC signature {found:#010x}, expected {expected:#010x}")
            }
            Self::PromoteEntryCount { declared, max } => {
                write!(
                    f,
                    "PROMOTE_CTX declared entryCount {declared} against a {max}-element promoteEntry[]"
                )
            }
            Self::PromoteAperture { entry, phys_attr } => {
                write!(
                    f,
                    "PROMOTE_CTX entry {entry}: physAttr {phys_attr:#010x} names undefined aperture 3"
                )
            }
            Self::PromoteLegacyShape {
                h_virt_memory,
                virt_address,
                size,
            } => {
                write!(
                    f,
                    "PROMOTE_CTX used the legacy path (hVirtMemory {h_virt_memory:#010x}, \
                     virtAddress {virt_address:#x}, size {size:#x})"
                )
            }
        }
    }
}

impl core::error::Error for AbiError {}

/// One field of a generated wire struct, as the **generator** computed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The C field name exactly as spelled in the header (`hClient`).
    pub c_name: &'static str,
    /// The Rust field name (`h_client`).
    pub rust_name: &'static str,
    /// Byte offset within the struct.
    pub offset: usize,
    /// Total width in bytes.
    pub width: usize,
}

/// A generated wire struct's layout, as the **generator** computed it.
///
/// Paired with the same struct's `RUSTC_OFFSETS` (built with `offset_of!`), this
/// is one of two independent computations of the same layout; the crate's tests
/// assert they agree. A single computation checked against itself would not be a
/// check at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructLayout {
    /// The C typedef name.
    pub c_name: &'static str,
    /// `sizeof`.
    pub size: usize,
    /// `alignof`.
    pub align: usize,
    /// Fields in declaration order.
    pub fields: &'static [Field],
}

impl StructLayout {
    /// Offset of a field by its Rust name, or `None` if this struct has no such
    /// field. Used by the oracle tests to name fields rather than indices.
    #[must_use]
    pub fn offset_of(&self, rust_name: &str) -> Option<usize> {
        self.fields
            .iter()
            .find(|f| f.rust_name == rust_name)
            .map(|f| f.offset)
    }
}

macro_rules! le_reader {
    ($name:ident, $ty:ty, $width:literal, $doc:literal) => {
        #[doc = $doc]
        ///
        /// # Errors
        ///
        /// [`AbiError::OutOfRange`] if the read would leave the buffer. Total on
        /// every input: no panic, no wrapping arithmetic.
        pub fn $name(bytes: &[u8], offset: usize) -> Result<$ty, AbiError> {
            let end = offset.checked_add($width).ok_or(AbiError::OutOfRange {
                offset,
                width: $width,
                len: bytes.len(),
            })?;
            let slice = bytes.get(offset..end).ok_or(AbiError::OutOfRange {
                offset,
                width: $width,
                len: bytes.len(),
            })?;
            let mut buf = [0u8; $width];
            buf.copy_from_slice(slice);
            Ok(<$ty>::from_le_bytes(buf))
        }
    };
}

le_reader!(u8_at, u8, 1, "Read a little-endian `u8` at `offset`.");
le_reader!(u16_at, u16, 2, "Read a little-endian `u16` at `offset`.");
le_reader!(u32_at, u32, 4, "Read a little-endian `u32` at `offset`.");
le_reader!(u64_at, u64, 8, "Read a little-endian `u64` at `offset`.");

// The NVIDIA ioctl/RPC ABI is host-endian, and every platform this project
// targets (x86_64, aarch64) is little-endian. The readers above are hard-coded
// LE, so a big-endian build would be wrong in a way no test on this box could
// see. Fail the BUILD instead.
const _: () = {
    assert!(
        cfg!(target_endian = "little"),
        "kayfabe-abi decodes host-endian NVIDIA structs and hard-codes little-endian"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The readers are exact at every boundary, and the last legal read is the
    /// one that ends exactly at `len`.
    #[test]
    fn le_readers_are_exact_and_bounded() {
        let b = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        assert_eq!(u8_at(&b, 0), Ok(0x11));
        assert_eq!(u8_at(&b, 7), Ok(0x88));
        assert_eq!(u16_at(&b, 0), Ok(0x2211));
        assert_eq!(u16_at(&b, 6), Ok(0x8877));
        assert_eq!(u32_at(&b, 0), Ok(0x4433_2211));
        assert_eq!(u32_at(&b, 4), Ok(0x8877_6655));
        assert_eq!(u64_at(&b, 0), Ok(0x8877_6655_4433_2211));
    }

    /// One byte past the end is refused with the exact numbers, not a panic and
    /// not a zero.
    #[test]
    fn le_readers_refuse_one_byte_past_the_end() {
        let b = [0u8; 8];
        assert_eq!(
            u8_at(&b, 8),
            Err(AbiError::OutOfRange {
                offset: 8,
                width: 1,
                len: 8
            })
        );
        assert_eq!(
            u16_at(&b, 7),
            Err(AbiError::OutOfRange {
                offset: 7,
                width: 2,
                len: 8
            })
        );
        assert_eq!(
            u32_at(&b, 5),
            Err(AbiError::OutOfRange {
                offset: 5,
                width: 4,
                len: 8
            })
        );
        assert_eq!(
            u64_at(&b, 1),
            Err(AbiError::OutOfRange {
                offset: 1,
                width: 8,
                len: 8
            })
        );
        assert_eq!(
            u64_at(&[], 0),
            Err(AbiError::OutOfRange {
                offset: 0,
                width: 8,
                len: 0
            })
        );
    }

    /// An offset near `usize::MAX` must not wrap into a valid range. A guest
    /// never supplies an offset directly today, but the readers are the last
    /// line and they are total by construction.
    #[test]
    fn le_readers_do_not_wrap_at_usize_max() {
        let b = [0u8; 8];
        assert_eq!(
            u64_at(&b, usize::MAX),
            Err(AbiError::OutOfRange {
                offset: usize::MAX,
                width: 8,
                len: 8
            })
        );
        assert_eq!(
            u16_at(&b, usize::MAX - 1),
            Err(AbiError::OutOfRange {
                offset: usize::MAX - 1,
                width: 2,
                len: 8
            })
        );
    }

    /// `offset_of` is a lookup, not a scan that can report the wrong field.
    #[test]
    fn struct_layout_offset_of_names_the_field() {
        const L: StructLayout = StructLayout {
            c_name: "T",
            size: 8,
            align: 4,
            fields: &[
                Field {
                    c_name: "a",
                    rust_name: "a",
                    offset: 0,
                    width: 4,
                },
                Field {
                    c_name: "bB",
                    rust_name: "b_b",
                    offset: 4,
                    width: 4,
                },
            ],
        };
        assert_eq!(L.offset_of("a"), Some(0));
        assert_eq!(L.offset_of("b_b"), Some(4));
        assert_eq!(
            L.offset_of("bB"),
            None,
            "the lookup is by RUST name, not C name"
        );
        assert_eq!(L.offset_of("nope"), None);
    }

    /// Every error renders with its numbers — a message that drops them makes a
    /// production log useless exactly when it matters.
    #[test]
    fn errors_render_their_numbers() {
        let t = AbiError::Truncated {
            c_name: "NVOS46_PARAMETERS",
            need: 64,
            got: 56,
        };
        assert_eq!(t.to_string(), "NVOS46_PARAMETERS: need 64 bytes, got 56");
        let v = AbiError::NoTableForVersion {
            major: 470,
            minor: 1,
            patch: 2,
        };
        assert_eq!(v.to_string(), "no ABI table for driver 470.1.2");
        let r = AbiError::RpcLength {
            declared: 0xFFFF_FFFF,
            available: 4096,
        };
        assert_eq!(
            r.to_string(),
            "RPC declared length 4294967295 against 4096 available bytes"
        );
        let s = AbiError::RpcSignature {
            found: 0,
            expected: 0x4350_5256,
        };
        assert_eq!(
            s.to_string(),
            "RPC signature 0x00000000, expected 0x43505256"
        );
        let o = AbiError::OutOfRange {
            offset: 9,
            width: 4,
            len: 8,
        };
        assert_eq!(
            o.to_string(),
            "read of 4 bytes at +9 exceeds the 8-byte buffer"
        );
    }
}
