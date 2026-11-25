//! ★★★ What [`crate::CharDevice::ioctl`] re-derives before the syscall that a request number does
//! not say by itself: the payload bound of a request that encodes no size (B-01), and the
//! **companion size field** of every pointer it mints (B-02 = stage-1 S1-40).
//!
//! Security review 2 (2026-11-13, `docs/audits/`): both were safe-caller contracts. A safe caller
//! could hand the funnel `FIONREAD` with a 1-byte slice (the kernel wrote 4 bytes: MEASURED, 3
//! outside the slice), or an `NVOS54` whose `paramsSize` exceeded the buffer its `params` pointer
//! names (RM `copy_to_user`s `paramsSize` bytes at the pointer: CODE READ). The owner rule is that
//! a wrong or hostile safe caller may cause a refusal or a panic, never memory corruption, so the
//! funnel re-derives both from the argument itself.
//!
//! It lives in a SAFE file on purpose: everything here is a pure predicate over integers and byte
//! slices, so each rule is unit-testable without a device, and the unsafe block in
//! `chardev_unsafe.rs` calls it before the first byte of the argument is patched.
//!
//! ## ⚠ This file holds driver vocabulary, and says so
//!
//! `chardev_unsafe.rs` used to promise "no NVIDIA vocabulary": the funnel would compile unchanged
//! against any driver. That promise is exactly why the size fields were a caller contract: only
//! the ABI knows where the count for a pointer lives. The vocabulary is kept to the [`RULES`]
//! table (four rows of offsets) and the funnel stays closed: an [`crate::Indirect`] on a request
//! with no row is REFUSED, so a new escape cannot gain a pointer without gaining a checked row.
//!
//! The offsets are the `kf-abi` encoders' (`kf_abi::generated::nvos::Nvos21Parameters` /
//! `Nvos54Parameters`, `kf_abi::bringup::Nvos02ParametersWithFd`,
//! `kf_abi::submit::NvMemoryListAllocationParams`, `kf_abi::drmnv`); this crate may not depend on
//! `kf-abi`, so `crates/kf-host/tests` is where a drift between the two would show (the callers
//! build with those encoders and these rules refuse a mismatch on the first ioctl).

use crate::error::RawError;
use crate::ioctl::{declared_size, magic_of, nr_of};

/// `_IOC_DIR` of a request word: bits 30-31.
fn dir_of(request: u64) -> u64 {
    (request >> 30) & 3
}

/// ★ B-01: requests whose number carries no `_IOC_SIZE`, with the bytes the kernel copies for
/// them. Anything with `_IOC_SIZE == 0` not listed here is refused.
///
/// `FIONREAD` (`0x541B`): the kernel writes one `int` (`fs/ioctl.c`, `put_user(.., (int __user *)
/// argp)`). The only size-less request any caller has issued (`chardev_unsafe.rs`'s own legacy test
/// and the frozen `export_backing.rs`).
pub(crate) const LEGACY_PAYLOAD: &[(u64, usize)] = &[(0x541B, core::mem::size_of::<libc::c_int>())];

/// ★★ B-01: the number of bytes the kernel copies at the argument for `request`, re-derived from
/// the number alone — or a refusal when the number does not say.
///
/// # Errors
/// [`RawError::IoctlSizeUnknown`]: bits above 31 (the kernel truncates `cmd` to 32 bits, so the
/// word would name a different request than the one the bound was derived from); a non-zero
/// `_IOC_SIZE` with no direction bits (not an `_IOC` encoding: a legacy driver number whose bits
/// merely decode as a size, e.g. the UVM numbers); or `_IOC_SIZE == 0` outside [`LEGACY_PAYLOAD`].
pub(crate) fn payload_bound(request: u64) -> Result<usize, RawError> {
    if request > u64::from(u32::MAX) {
        return Err(RawError::IoctlSizeUnknown { request });
    }
    let size = declared_size(request);
    if size != 0 {
        if dir_of(request) == 0 {
            return Err(RawError::IoctlSizeUnknown { request });
        }
        return Ok(size);
    }
    LEGACY_PAYLOAD
        .iter()
        .find(|(r, _)| *r == request)
        .map(|(_, n)| *n)
        .ok_or(RawError::IoctlSizeUnknown { request })
}

/// Width of a size field.
#[derive(Debug, Clone, Copy)]
enum Width {
    U32,
    U64,
}

impl Width {
    fn bytes(self) -> usize {
        match self {
            Width::U32 => 4,
            Width::U64 => 8,
        }
    }
}

/// A nested pointer the [`Rule`] covers: inside the outer buffer, at `inner_at`, a pointer to an
/// array of `count_at`'s value elements of `elem` bytes — valid only when the argument's class
/// word at `class_at` is one of `classes` (the class decides how RM reads the outer buffer).
#[derive(Debug)]
struct NestedRule {
    class_at: usize,
    classes: &'static [u32],
    inner_at: usize,
    count_at: usize,
    elem: u64,
}

/// One request whose argument has a pointer field: where it is, and where the count the driver
/// copies at it is (`bytes = field + bias`).
#[derive(Debug)]
struct Rule {
    magic: u8,
    nr: u8,
    size: usize,
    ptr_at: usize,
    size_at: usize,
    width: Width,
    bias: u64,
    nested: Option<NestedRule>,
}

/// `NV01_MEMORY_LIST_SYSTEM`, `_FBMEM`, `_OBJECT` (`cl84a0.h`): the classes whose allocation
/// parameters (`NV_MEMORY_LIST_ALLOCATION_PARAMS`) carry `pageNumberList`.
const MEMORY_LIST_CLASSES: &[u32] = &[0x81, 0x82, 0x83];

/// ★★★ The rows. A request not listed here may carry NO [`crate::Indirect`].
const RULES: &[Rule] = &[
    // NV_ESC_RM_CONTROL, NVOS54 (32 bytes): `params` @16, `paramsSize` (u32) @24.
    Rule {
        magic: b'F',
        nr: 0x2A,
        size: 32,
        ptr_at: 16,
        size_at: 24,
        width: Width::U32,
        bias: 0,
        nested: None,
    },
    // NV_ESC_RM_ALLOC, NVOS21 (32 bytes): `pAllocParms` @16, `paramsSize` (u32) @24; nested:
    // NV_MEMORY_LIST_ALLOCATION_PARAMS `pageNumberList` @128, `pageCount` (u32) @68, NvU64 each.
    Rule {
        magic: b'F',
        nr: 0x2B,
        size: 32,
        ptr_at: 16,
        size_at: 24,
        width: Width::U32,
        bias: 0,
        nested: Some(NestedRule {
            class_at: 12,
            classes: MEMORY_LIST_CLASSES,
            inner_at: 128,
            count_at: 68,
            elem: 8,
        }),
    },
    // NV_ESC_RM_ALLOC_MEMORY, nv_ioctl_nvos02_parameters_with_fd (56 bytes): `pMemory` @24,
    // `limit` (u64) @32 — a LIMIT, so the bytes are `limit + 1`.
    Rule {
        magic: b'F',
        nr: 0x27,
        size: 56,
        ptr_at: 24,
        size_at: 32,
        width: Width::U64,
        bias: 1,
        nested: None,
    },
    // DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY (32 bytes): `nvkms_params_ptr` @8,
    // `nvkms_params_size` (u64) @16.
    Rule {
        magic: b'd',
        nr: 0x41,
        size: 32,
        ptr_at: 8,
        size_at: 16,
        width: Width::U64,
        bias: 0,
        nested: None,
    },
];

fn rule_for(request: u64) -> Option<&'static Rule> {
    let (magic, nr, size) = (magic_of(request), nr_of(request), declared_size(request));
    RULES
        .iter()
        .find(|r| r.magic == magic && r.nr == nr && r.size == size)
}

/// Read an unsigned little-endian field; `None` when it does not fit `bytes`.
fn field(bytes: &[u8], at: usize, width: Width) -> Option<u64> {
    let b = bytes.get(at..at.checked_add(width.bytes())?)?;
    Some(match width {
        Width::U32 => u64::from(u32::from_le_bytes(b.try_into().ok()?)),
        Width::U64 => u64::from_le_bytes(b.try_into().ok()?),
    })
}

/// What the funnel tells the check about one [`crate::Indirect`] — lengths and offsets only, never
/// an address.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PatchView<'a> {
    /// The pointer field's offset in the argument.
    pub at: usize,
    /// Bytes the pointed-at range holds.
    pub len: u64,
    /// The pointed-at bytes, when they are a buffer (a described region has none to read).
    pub bytes: Option<&'a [u8]>,
    /// A nested pointer: (offset inside `bytes`, bytes of the inner buffer).
    pub nested: Option<(usize, u64)>,
}

/// ★★★ B-02: every pointer the funnel will mint has a companion size field that fits its buffer,
/// and no known pointer field is left non-zero without a buffer behind it.
///
/// # Errors
/// [`RawError::UnvalidatedPointer`] — an [`crate::Indirect`] on a request with no [`RULES`] row,
/// at an offset the row does not name, a nested pointer the row does not cover, or a known pointer
/// field carrying a value no [`crate::Indirect`] covers. [`RawError::IndirectSizeMismatch`] — a
/// size field (after its bias, with checked arithmetic) states more bytes than the buffer holds.
pub(crate) fn check(request: u64, arg: &[u8], patches: &[PatchView<'_>]) -> Result<(), RawError> {
    let refuse = |at: usize, why: &'static str| RawError::UnvalidatedPointer {
        request,
        at: at as u64,
        why,
    };
    let Some(rule) = rule_for(request) else {
        return match patches.first() {
            Some(p) => Err(refuse(
                p.at,
                "the request has no companion-size rule, so no pointer may be minted for it",
            )),
            None => Ok(()),
        };
    };
    let mut covered = false;
    for p in patches {
        if p.at != rule.ptr_at {
            return Err(refuse(
                p.at,
                "the request's rule names no pointer field here",
            ));
        }
        covered = true;
        let stated = field(arg, rule.size_at, rule.width)
            .ok_or_else(|| refuse(p.at, "the size field leaves the argument"))?
            .checked_add(rule.bias)
            .ok_or(RawError::IndirectSizeMismatch {
                pointer_at: p.at as u64,
                stated: u64::MAX,
                buffer: p.len,
            })?;
        if stated > p.len {
            return Err(RawError::IndirectSizeMismatch {
                pointer_at: p.at as u64,
                stated,
                buffer: p.len,
            });
        }
        if let Some((inner_at, inner_len)) = p.nested {
            let n = rule
                .nested
                .as_ref()
                .ok_or_else(|| refuse(inner_at, "the request's rule covers no nested pointer"))?;
            if inner_at != n.inner_at {
                return Err(refuse(inner_at, "the rule names no nested pointer here"));
            }
            let class = field(arg, n.class_at, Width::U32)
                .ok_or_else(|| refuse(inner_at, "the class word leaves the argument"))?;
            if !n.classes.iter().any(|c| u64::from(*c) == class) {
                return Err(refuse(
                    inner_at,
                    "the argument's class does not carry this nested pointer",
                ));
            }
            let outer = p
                .bytes
                .ok_or_else(|| refuse(inner_at, "a described region carries no nested pointer"))?;
            let count = field(outer, n.count_at, Width::U32)
                .ok_or_else(|| refuse(inner_at, "the element count leaves the outer buffer"))?;
            let need = count
                .checked_mul(n.elem)
                .ok_or(RawError::IndirectSizeMismatch {
                    pointer_at: inner_at as u64,
                    stated: u64::MAX,
                    buffer: inner_len,
                })?;
            if need > inner_len {
                return Err(RawError::IndirectSizeMismatch {
                    pointer_at: inner_at as u64,
                    stated: need,
                    buffer: inner_len,
                });
            }
        }
    }
    if !covered {
        let ptr = field(arg, rule.ptr_at, Width::U64)
            .ok_or_else(|| refuse(rule.ptr_at, "the pointer field leaves the argument"))?;
        if ptr != 0 {
            return Err(refuse(
                rule.ptr_at,
                "a non-zero value sits in a pointer field no Indirect covers",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ioctl;

    fn ctl() -> u64 {
        ioctl::readwrite(b'F', 0x2A, 32).expect("32 fits")
    }

    fn arg_with_size(size_at: usize, v: u32) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[size_at..size_at + 4].copy_from_slice(&v.to_le_bytes());
        a
    }

    fn buf(at: usize, len: u64) -> PatchView<'static> {
        PatchView {
            at,
            len,
            bytes: None,
            nested: None,
        }
    }

    // ---- B-01 ----

    #[test]
    fn a_size_less_request_outside_the_legacy_table_is_refused() {
        // TCGETS: `_IOC_SIZE` 0, the kernel writes a 36-byte termios.
        assert_eq!(
            payload_bound(0x5401),
            Err(RawError::IoctlSizeUnknown { request: 0x5401 })
        );
    }

    #[test]
    fn fionread_is_bounded_by_its_table_entry() {
        assert_eq!(payload_bound(0x541B), Ok(4));
    }

    #[test]
    fn a_sized_number_without_direction_bits_is_not_an_ioc_encoding() {
        // UVM_INITIALIZE-style magic number: bits decode as a 12288-byte size, dir NONE.
        assert_eq!(
            payload_bound(0x3000_0001),
            Err(RawError::IoctlSizeUnknown {
                request: 0x3000_0001
            })
        );
    }

    #[test]
    fn bits_above_31_are_refused() {
        let r = ctl() | (1 << 32);
        assert_eq!(
            payload_bound(r),
            Err(RawError::IoctlSizeUnknown { request: r })
        );
    }

    #[test]
    fn the_largest_32_bit_word_is_still_a_request() {
        // dir 3, size 0x3FFF: the edge of the `> u32::MAX` refusal, which must stay `>`.
        assert_eq!(payload_bound(0xFFFF_FFFF), Ok(crate::ioctl::MAX_IOCTL_SIZE));
    }

    #[test]
    fn an_ioc_encoded_request_is_bounded_by_its_size_field() {
        assert_eq!(payload_bound(ctl()), Ok(32));
        assert_eq!(payload_bound(ioctl::read(b'F', 1, 7).expect("fits")), Ok(7));
        assert_eq!(
            payload_bound(ioctl::write(b'F', 1, 9).expect("fits")),
            Ok(9)
        );
    }

    // ---- B-02 ----

    #[test]
    fn params_size_larger_than_the_buffer_is_refused() {
        let a = arg_with_size(24, 4096);
        assert_eq!(
            check(ctl(), &a, &[buf(16, 32)]),
            Err(RawError::IndirectSizeMismatch {
                pointer_at: 16,
                stated: 4096,
                buffer: 32,
            })
        );
    }

    #[test]
    fn params_size_one_past_the_buffer_is_refused_and_equal_passes() {
        let a = arg_with_size(24, 33);
        assert!(matches!(
            check(ctl(), &a, &[buf(16, 32)]),
            Err(RawError::IndirectSizeMismatch { stated: 33, .. })
        ));
        let a = arg_with_size(24, 32);
        assert_eq!(check(ctl(), &a, &[buf(16, 32)]), Ok(()));
        let a = arg_with_size(24, 31);
        assert_eq!(check(ctl(), &a, &[buf(16, 32)]), Ok(()));
    }

    #[test]
    fn nvos02_limit_is_a_limit_and_its_plus_one_is_checked() {
        let req = ioctl::readwrite(b'F', 0x27, 56).expect("56 fits");
        let mut a = [0u8; 56];
        a[32..40].copy_from_slice(&4095u64.to_le_bytes());
        assert_eq!(check(req, &a, &[buf(24, 4096)]), Ok(()), "limit = len - 1");
        a[32..40].copy_from_slice(&4096u64.to_le_bytes());
        assert!(matches!(
            check(req, &a, &[buf(24, 4096)]),
            Err(RawError::IndirectSizeMismatch { stated: 4097, .. })
        ));
        a[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(
            matches!(
                check(req, &a, &[buf(24, 4096)]),
                Err(RawError::IndirectSizeMismatch {
                    stated: u64::MAX,
                    ..
                })
            ),
            "limit + 1 overflows: refused, never wrapped to 0"
        );
    }

    #[test]
    fn an_indirect_on_a_request_without_a_rule_is_refused() {
        let req = ioctl::readwrite(b'F', 0x99, 32).expect("fits");
        assert!(matches!(
            check(req, &[0u8; 32], &[buf(16, 32)]),
            Err(RawError::UnvalidatedPointer { at: 16, .. })
        ));
        assert_eq!(
            check(req, &[0u8; 32], &[]),
            Ok(()),
            "no pointer, nothing to check"
        );
    }

    #[test]
    fn an_indirect_at_an_offset_the_rule_does_not_name_is_refused() {
        assert!(matches!(
            check(ctl(), &[0u8; 32], &[buf(8, 32)]),
            Err(RawError::UnvalidatedPointer { at: 8, .. })
        ));
    }

    #[test]
    fn a_wrong_declared_size_does_not_match_the_row() {
        // NV_ESC_RM_CONTROL with a 48-byte argument is not NVOS54: no row, no pointer.
        let req = ioctl::readwrite(b'F', 0x2A, 48).expect("fits");
        assert!(matches!(
            check(req, &[0u8; 48], &[buf(16, 32)]),
            Err(RawError::UnvalidatedPointer { .. })
        ));
    }

    #[test]
    fn a_non_zero_uncovered_pointer_field_is_refused() {
        let mut a = [0u8; 32];
        a[16..24].copy_from_slice(&0xdead_beefu64.to_le_bytes());
        assert!(matches!(
            check(ctl(), &a, &[]),
            Err(RawError::UnvalidatedPointer { at: 16, .. })
        ));
        assert_eq!(
            check(ctl(), &[0u8; 32], &[]),
            Ok(()),
            "a null pointer passes"
        );
    }

    fn alloc_arg(class: u32, params_size: u32) -> [u8; 32] {
        let mut a = arg_with_size(24, params_size);
        a[12..16].copy_from_slice(&class.to_le_bytes());
        a
    }

    fn list_params(count: u32) -> [u8; 152] {
        let mut p = [0u8; 152];
        p[68..72].copy_from_slice(&count.to_le_bytes());
        p
    }

    #[test]
    fn a_nested_list_count_is_checked_against_the_inner_buffer() {
        let req = ioctl::readwrite(b'F', 0x2B, 32).expect("fits");
        let a = alloc_arg(0x83, 152);
        let p = list_params(3);
        let view = |inner_len| PatchView {
            at: 16,
            len: 152,
            bytes: Some(&p),
            nested: Some((128, inner_len)),
        };
        assert_eq!(check(req, &a, &[view(24)]), Ok(()), "3 x 8 = 24");
        assert_eq!(
            check(req, &a, &[view(23)]),
            Err(RawError::IndirectSizeMismatch {
                pointer_at: 128,
                stated: 24,
                buffer: 23,
            })
        );
    }

    #[test]
    fn a_nested_count_that_overflows_is_refused() {
        // u32::MAX * 8 does not overflow u64, so the overflow arm is the multiply's guard; the
        // check that matters is the bound itself.
        let req = ioctl::readwrite(b'F', 0x2B, 32).expect("fits");
        let a = alloc_arg(0x81, 152);
        let p = list_params(u32::MAX);
        let v = PatchView {
            at: 16,
            len: 152,
            bytes: Some(&p),
            nested: Some((128, 4096)),
        };
        assert!(matches!(
            check(req, &a, &[v]),
            Err(RawError::IndirectSizeMismatch {
                pointer_at: 128,
                ..
            })
        ));
    }

    #[test]
    fn a_nested_pointer_needs_a_memory_list_class_and_the_named_offset() {
        let req = ioctl::readwrite(b'F', 0x2B, 32).expect("fits");
        let p = list_params(0);
        let v = |at| PatchView {
            at: 16,
            len: 152,
            bytes: Some(&p),
            nested: Some((at, 8)),
        };
        assert!(matches!(
            check(req, &alloc_arg(0x3e, 152), &[v(128)]),
            Err(RawError::UnvalidatedPointer { at: 128, .. })
        ));
        assert!(matches!(
            check(req, &alloc_arg(0x83, 152), &[v(120)]),
            Err(RawError::UnvalidatedPointer { at: 120, .. })
        ));
        assert!(
            matches!(
                check(ctl(), &arg_with_size(24, 152), &[v(128)]),
                Err(RawError::UnvalidatedPointer { at: 128, .. })
            ),
            "RM_CONTROL has no nested rule"
        );
    }

    #[test]
    fn the_drm_import_row_checks_its_u64_size() {
        let req = 0xC020_6441u64;
        let mut a = [0u8; 32];
        a[16..24].copy_from_slice(&28u64.to_le_bytes());
        assert_eq!(check(req, &a, &[buf(8, 28)]), Ok(()));
        a[16..24].copy_from_slice(&29u64.to_le_bytes());
        assert!(matches!(
            check(req, &a, &[buf(8, 28)]),
            Err(RawError::IndirectSizeMismatch { stated: 29, .. })
        ));
    }
}
