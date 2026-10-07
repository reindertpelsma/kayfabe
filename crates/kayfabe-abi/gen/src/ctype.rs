//! The NVIDIA SDK scalar type table and the C struct-layout algorithm.
//!
//! # Why this is target-independent (the arm64-vs-x86_64 answer)
//!
//! Every field in the NVIDIA SDK headers is declared with a **fixed-width** NVIDIA
//! typedef, never a C primitive whose width varies (`long`, `size_t`, a pointer).
//! The one apparent exception is `NvP64`, and it is not one:
//!
//! ```c
//! // ogkm src/common/sdk/nvidia/inc/nvtypes.h:304-326
//! #if defined(NV_64_BITS)
//! typedef void*  NvP64;   /* 64 bit void pointer */
//! #else
//! typedef NvU64  NvP64;   /* 64 bit void pointer */
//! #endif
//! ```
//!
//! — 8 bytes on both arms. `NV_ALIGN_BYTES(8)` / `NV_DECLARE_ALIGNED(x, 8)` then
//! force 8-byte alignment on the ILP32 platforms where a 64-bit scalar would
//! otherwise be 4-aligned. So on **every** LP64 target (x86_64, aarch64) the
//! alignment attributes are no-ops and the layout is identical.
//!
//! That is a derivation, not an assumption, and the generator enforces it: an
//! alignment attribute that would *raise* a field's alignment above its natural
//! alignment is a hard error ([`LayoutError::AlignmentRaises`]), because such a
//! field would need `#[repr(C, align(N))]` in the emitted Rust and would make the
//! plain-`#[repr(C)]` mirror wrong. As of ogkm 610.43.02 no struct in the slice
//! trips it; the day one does, the generator stops rather than emitting a lie.

use std::fmt;

/// A scalar C type from the NVIDIA SDK headers: how wide it is, how it aligns,
/// and what Rust spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scalar {
    /// The C spelling as it appears in the header.
    pub c_name: &'static str,
    /// Width in bytes.
    pub size: usize,
    /// Natural alignment in bytes (equals `size` for every NVIDIA scalar).
    pub align: usize,
    /// The Rust type the emitter uses.
    pub rust: &'static str,
    /// The little-endian read/write helper suffix (`u8`, `u16`, `u32`, `u64`).
    pub prim: &'static str,
}

/// The complete scalar table.
///
/// **Deliberately closed.** An unrecognised type is a hard error, never a guessed
/// width — the L11 bug class (`abi_struct_truncation`, `nvos64_abi_fix`) is
/// precisely what a guessed width produces.
pub const SCALARS: &[Scalar] = &[
    Scalar {
        c_name: "NvU8",
        size: 1,
        align: 1,
        rust: "u8",
        prim: "u8",
    },
    Scalar {
        c_name: "NvS8",
        size: 1,
        align: 1,
        rust: "i8",
        prim: "u8",
    },
    Scalar {
        c_name: "NvBool",
        size: 1,
        align: 1,
        rust: "u8",
        prim: "u8",
    },
    Scalar {
        c_name: "char",
        size: 1,
        align: 1,
        rust: "u8",
        prim: "u8",
    },
    Scalar {
        c_name: "NvU16",
        size: 2,
        align: 2,
        rust: "u16",
        prim: "u16",
    },
    Scalar {
        c_name: "NvS16",
        size: 2,
        align: 2,
        rust: "i16",
        prim: "u16",
    },
    Scalar {
        c_name: "NvV16",
        size: 2,
        align: 2,
        rust: "u16",
        prim: "u16",
    },
    Scalar {
        c_name: "NvU32",
        size: 4,
        align: 4,
        rust: "u32",
        prim: "u32",
    },
    Scalar {
        c_name: "NvS32",
        size: 4,
        align: 4,
        rust: "i32",
        prim: "u32",
    },
    Scalar {
        c_name: "NvV32",
        size: 4,
        align: 4,
        rust: "u32",
        prim: "u32",
    },
    Scalar {
        c_name: "NvHandle",
        size: 4,
        align: 4,
        rust: "u32",
        prim: "u32",
    },
    Scalar {
        c_name: "NvU64",
        size: 8,
        align: 8,
        rust: "u64",
        prim: "u64",
    },
    Scalar {
        c_name: "NvS64",
        size: 8,
        align: 8,
        rust: "i64",
        prim: "u64",
    },
    Scalar {
        c_name: "NvP64",
        size: 8,
        align: 8,
        rust: "u64",
        prim: "u64",
    },
    Scalar {
        c_name: "NvLength",
        size: 8,
        align: 8,
        rust: "u64",
        prim: "u64",
    },
];

/// Look a C type spelling up in [`SCALARS`].
#[must_use]
pub fn scalar(c_name: &str) -> Option<&'static Scalar> {
    SCALARS.iter().find(|s| s.c_name == c_name)
}

/// Look a C type spelling up in a manifest-declared extension table first, then
/// in [`SCALARS`]. The extension table exists for the small aggregates NVIDIA
/// embeds by value (the RPC header's 4-byte union); every entry is verified
/// against its own definition in the header, so it is a *declaration*, not a
/// guess.
#[must_use]
pub fn scalar_with(extra: &'static [Scalar], c_name: &str) -> Option<&'static Scalar> {
    extra
        .iter()
        .find(|s| s.c_name == c_name)
        .or_else(|| scalar(c_name))
}

/// One laid-out field of a struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The C field name exactly as written (`hClient`).
    pub c_name: String,
    /// The snake_case Rust field name (`h_client`).
    pub rust_name: String,
    /// The scalar element type.
    pub scalar: &'static Scalar,
    /// `Some(n)` for `T name[n]`; `None` for a plain scalar.
    pub array_len: Option<usize>,
    /// Byte offset within the struct, as computed by [`lay_out`].
    pub offset: usize,
    /// Total width in bytes (`scalar.size * array_len.unwrap_or(1)`).
    pub width: usize,
    /// Source line in the originating header (1-based), for provenance comments.
    pub line: usize,
}

/// A struct whose layout has been computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// C typedef name (`NVOS46_PARAMETERS`).
    pub c_name: String,
    /// Rust type name (`Nvos46Parameters`).
    pub rust_name: String,
    /// `sizeof` in bytes, including tail padding.
    pub size: usize,
    /// `alignof` in bytes.
    pub align: usize,
    /// Fields in declaration order.
    pub fields: Vec<Field>,
    /// A trailing flexible-array member (`T name[]`), if present. It contributes
    /// its alignment but **no** size — exactly C's rule.
    pub flexible_tail: Option<String>,
    /// Header path relative to the ogkm root.
    pub header: String,
    /// Line of the `typedef` (1-based).
    pub line: usize,
}

/// Reasons the generator refuses to emit rather than guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    /// A type spelling that is not in the closed [`SCALARS`] table.
    UnknownType {
        /// The unrecognised C type spelling.
        ty: String,
        /// The field it appeared on.
        field: String,
    },
    /// An alignment attribute that raises a field above its natural alignment.
    /// Emitting a plain `#[repr(C)]` mirror would then be wrong.
    AlignmentRaises {
        /// The field carrying the attribute.
        field: String,
        /// The requested alignment.
        requested: usize,
        /// The type's natural alignment.
        natural: usize,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownType { ty, field } => {
                write!(
                    f,
                    "unknown C type `{ty}` on field `{field}` (add it to SCALARS — never guess)"
                )
            }
            Self::AlignmentRaises {
                field,
                requested,
                natural,
            } => write!(
                f,
                "field `{field}` requests align {requested} above its natural align {natural}; \
                 a plain #[repr(C)] mirror would be wrong — emit #[repr(C, align(N))] deliberately"
            ),
        }
    }
}

/// Round `n` up to a multiple of `align` (which must be a power of two ≥ 1).
#[must_use]
pub fn align_up(n: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (n + align - 1) & !(align - 1)
}

/// A field as parsed, before layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawField {
    /// C type spelling.
    pub ty: String,
    /// C field name.
    pub name: String,
    /// `Some(n)` for `[n]`, `None` for a scalar.
    pub array_len: Option<usize>,
    /// Explicit alignment from `NV_ALIGN_BYTES` / `NV_DECLARE_ALIGNED`, if any.
    pub explicit_align: Option<usize>,
    /// 1-based source line.
    pub line: usize,
}

/// Apply the C struct-layout algorithm.
///
/// This is the generator's **own** implementation. The emitted Rust mirrors the
/// same field list under a plain `#[repr(C)]`, so rustc computes the layout a
/// second time by its own algorithm; the generated `LAYOUT` table and rustc's
/// `size_of`/`offset_of!` are then asserted equal in the crate's tests. Two
/// independent computations agreeing is the check — a single computation checked
/// against itself would not be one.
///
/// # Errors
///
/// [`LayoutError::UnknownType`] for a type outside [`SCALARS`];
/// [`LayoutError::AlignmentRaises`] for an alignment attribute that would need
/// `#[repr(C, align(N))]`.
pub fn lay_out(
    c_name: &str,
    header: &str,
    line: usize,
    raw: &[RawField],
    flexible_tail: Option<(String, usize)>,
    extra: &'static [Scalar],
) -> Result<Layout, LayoutError> {
    let mut cursor = 0usize;
    let mut struct_align = 1usize;
    let mut fields = Vec::with_capacity(raw.len());

    for rf in raw {
        let sc = scalar_with(extra, &rf.ty).ok_or_else(|| LayoutError::UnknownType {
            ty: rf.ty.clone(),
            field: rf.name.clone(),
        })?;
        if let Some(req) = rf.explicit_align
            && req > sc.align
        {
            return Err(LayoutError::AlignmentRaises {
                field: rf.name.clone(),
                requested: req,
                natural: sc.align,
            });
        }
        let width = sc.size * rf.array_len.unwrap_or(1);
        let offset = align_up(cursor, sc.align);
        cursor = offset + width;
        struct_align = struct_align.max(sc.align);
        fields.push(Field {
            c_name: rf.name.clone(),
            rust_name: snake_case(&rf.name),
            scalar: sc,
            array_len: rf.array_len,
            offset,
            width,
            line: rf.line,
        });
    }

    // A flexible array member contributes its element alignment to the struct's
    // alignment but occupies no space (C11 6.7.2.1p18).
    let mut tail_name = None;
    if let Some((name, elem_align)) = flexible_tail {
        struct_align = struct_align.max(elem_align);
        tail_name = Some(name);
    }

    Ok(Layout {
        c_name: c_name.to_string(),
        rust_name: pascal_case(c_name),
        size: align_up(cursor, struct_align),
        align: struct_align,
        fields,
        flexible_tail: tail_name,
        header: header.to_string(),
        line,
    })
}

/// Lay out a C `union`: every member starts at offset 0; `sizeof` is the widest
/// member rounded up to the widest alignment.
///
/// Used only to **verify** a manifest-declared aggregate scalar against its own
/// definition in the header, so a hand-declared size/align cannot drift.
///
/// # Errors
///
/// Same as [`lay_out`].
pub fn lay_out_union(
    raw: &[RawField],
    extra: &'static [Scalar],
) -> Result<(usize, usize), LayoutError> {
    let mut widest = 0usize;
    let mut align = 1usize;
    for rf in raw {
        let sc = scalar_with(extra, &rf.ty).ok_or_else(|| LayoutError::UnknownType {
            ty: rf.ty.clone(),
            field: rf.name.clone(),
        })?;
        if let Some(req) = rf.explicit_align
            && req > sc.align
        {
            return Err(LayoutError::AlignmentRaises {
                field: rf.name.clone(),
                requested: req,
                natural: sc.align,
            });
        }
        widest = widest.max(sc.size * rf.array_len.unwrap_or(1));
        align = align.max(sc.align);
    }
    Ok((align_up(widest, align), align))
}

/// `hClient` → `h_client`, `processID` → `process_id`, `vaSpaceSize` →
/// `va_space_size`, `pOsPidInfo` → `p_os_pid_info`.
///
/// Runs of capitals are treated as one word (`processID` → `process_id`, not
/// `process_i_d`), except that the final capital of a run starts a new word when
/// a lowercase letter follows it (`NVOSFoo` → `nvos_foo`).
#[must_use]
pub fn snake_case(c_name: &str) -> String {
    let chars: Vec<char> = c_name.chars().collect();
    let mut out = String::with_capacity(chars.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '_' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            continue;
        }
        if ch.is_ascii_uppercase() {
            let prev_lower_or_digit =
                i > 0 && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            let prev_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
            if !out.is_empty()
                && !out.ends_with('_')
                && (prev_lower_or_digit || (prev_upper && next_lower))
            {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `NVOS46_PARAMETERS` → `Nvos46Parameters`,
/// `rpc_message_header_v03_00` → `RpcMessageHeaderV0300`.
#[must_use]
pub fn pascal_case(c_name: &str) -> String {
    let mut out = String::with_capacity(c_name.len());
    for word in c_name.split('_').filter(|w| !w.is_empty()) {
        let lower = word.to_ascii_lowercase();
        let mut cs = lower.chars();
        if let Some(first) = cs.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(cs.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_handles_the_nvidia_spellings() {
        assert_eq!(snake_case("hClient"), "h_client");
        assert_eq!(snake_case("processID"), "process_id");
        assert_eq!(snake_case("vaSpaceSize"), "va_space_size");
        assert_eq!(snake_case("pOsPidInfo"), "p_os_pid_info");
        assert_eq!(snake_case("hObjectParent"), "h_object_parent");
        assert_eq!(snake_case("kindOverride"), "kind_override");
        assert_eq!(snake_case("header_version"), "header_version");
        assert_eq!(snake_case("rpc_result_private"), "rpc_result_private");
        assert_eq!(snake_case("deviceId"), "device_id");
        assert_eq!(snake_case("chId"), "ch_id");
        assert_eq!(snake_case("flags2"), "flags2");
        assert_eq!(snake_case("u"), "u");
    }

    #[test]
    fn pascal_case_handles_the_nvidia_spellings() {
        assert_eq!(pascal_case("NVOS46_PARAMETERS"), "Nvos46Parameters");
        assert_eq!(
            pascal_case("rpc_message_header_v03_00"),
            "RpcMessageHeaderV0300"
        );
        assert_eq!(
            pascal_case("NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS"),
            "Nv0080CtrlDmaSetPageDirectoryParams"
        );
    }

    /// The layout algorithm reproduces the padding NVOS46_V580 actually has:
    /// four handles, two 8-aligned u64s, three u32s, an 8-aligned u64 (which
    /// forces 4 bytes of padding after `kindOverride`), a u32, and 4 bytes of
    /// tail padding to the struct's 8-byte alignment. Total 64.
    #[test]
    fn lay_out_reproduces_nvos46_v580_padding() {
        let f = |ty: &str, name: &str| RawField {
            ty: ty.into(),
            name: name.into(),
            array_len: None,
            explicit_align: None,
            line: 0,
        };
        let raw = vec![
            f("NvHandle", "hClient"),
            f("NvHandle", "hDevice"),
            f("NvHandle", "hDma"),
            f("NvHandle", "hMemory"),
            RawField {
                explicit_align: Some(8),
                ..f("NvU64", "offset")
            },
            RawField {
                explicit_align: Some(8),
                ..f("NvU64", "length")
            },
            f("NvV32", "flags"),
            f("NvV32", "flags2"),
            f("NvV32", "kindOverride"),
            RawField {
                explicit_align: Some(8),
                ..f("NvU64", "dmaOffset")
            },
            f("NvV32", "status"),
        ];
        let l = lay_out("NVOS46_PARAMETERS", "h", 1, &raw, None, &[]).expect("lays out");
        let off = |n: &str| {
            l.fields
                .iter()
                .find(|f| f.c_name == n)
                .expect("field")
                .offset
        };
        assert_eq!(off("hClient"), 0);
        assert_eq!(off("hMemory"), 12);
        assert_eq!(off("offset"), 16);
        assert_eq!(off("length"), 24);
        assert_eq!(off("flags"), 32);
        assert_eq!(off("flags2"), 36);
        assert_eq!(off("kindOverride"), 40);
        assert_eq!(off("dmaOffset"), 48, "8-aligned, so +44 is padding");
        assert_eq!(off("status"), 56);
        assert_eq!(l.align, 8);
        assert_eq!(l.size, 64, "60 rounded up to the struct's 8-byte alignment");
    }

    /// An unknown type is refused by name — never silently sized.
    #[test]
    fn unknown_type_is_a_hard_error_naming_the_field() {
        let raw = vec![RawField {
            ty: "SomeAggregate".into(),
            name: "thing".into(),
            array_len: None,
            explicit_align: None,
            line: 7,
        }];
        assert_eq!(
            lay_out("X", "h", 1, &raw, None, &[]),
            Err(LayoutError::UnknownType {
                ty: "SomeAggregate".into(),
                field: "thing".into()
            })
        );
    }

    /// An alignment attribute that would RAISE alignment stops the generator,
    /// because the emitted plain `#[repr(C)]` mirror could not express it.
    #[test]
    fn alignment_that_raises_is_a_hard_error() {
        let raw = vec![RawField {
            ty: "NvU32".into(),
            name: "x".into(),
            array_len: None,
            explicit_align: Some(8),
            line: 7,
        }];
        assert_eq!(
            lay_out("X", "h", 1, &raw, None, &[]),
            Err(LayoutError::AlignmentRaises {
                field: "x".into(),
                requested: 8,
                natural: 4
            })
        );
    }

    /// A flexible array member adds alignment but no size — the rpc message
    /// header's `rpc_generic_union rpc_message_data[]` case.
    #[test]
    fn flexible_array_member_contributes_alignment_but_no_size() {
        let raw = vec![RawField {
            ty: "NvU32".into(),
            name: "a".into(),
            array_len: None,
            explicit_align: None,
            line: 1,
        }];
        let l = lay_out("X", "h", 1, &raw, Some(("tail".into(), 8)), &[]).expect("lays out");
        assert_eq!(
            l.align, 8,
            "the FAM's element alignment raises the struct's"
        );
        assert_eq!(
            l.size, 8,
            "4 bytes of field rounded up to align 8; the FAM adds none"
        );
        assert_eq!(l.flexible_tail.as_deref(), Some("tail"));
    }

    #[test]
    fn align_up_is_exact_at_the_boundaries() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
        assert_eq!(align_up(60, 8), 64);
        assert_eq!(align_up(7, 1), 7);
    }
}
