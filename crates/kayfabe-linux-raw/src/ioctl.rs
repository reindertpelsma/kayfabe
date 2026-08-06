//! The `asm-generic` ioctl **request-number encoding** — pure arithmetic, no syscall.
//!
//! `l1_os_shell.md` §4.1.1 asks the dangerous files to be small and boring, and §4.7 asks
//! this crate to hold no business logic. Encoding a request number is neither: it is an
//! **OS fact** (the layout of the 32-bit `_IOC` word) that every caller of
//! [`crate::CharDevice::ioctl`] would otherwise re-derive with shifts, and re-derived
//! shifts are how a driver ends up being handed `_IOR` where it wanted `_IOWR`.
//!
//! It lives in a safe file because none of it touches a pointer. The keyword appears
//! nowhere here, which is why the file is not named `*_unsafe.rs`.
//!
//! ## The one refusal
//!
//! The size field is **14 bits**. A request whose payload does not fit is a programming
//! error that the kernel would see as a *different, valid* request number — silently the
//! wrong ioctl — so it is [`RawError::TooLargeForHost`] here rather than a wrapping shift.
//!
//! ## Why `u64` and not `u32`
//!
//! `ioctl(2)`'s request argument is `unsigned long` on Linux. The encoded word is 32 bits;
//! widening at the boundary keeps the sign-extension question from ever arising at the
//! call site (`c_ulong` is 64-bit on both targets this crate builds for, and a `u32` that
//! sets bit 31 — every `_IOWR` does — is exactly the value an accidental `as i32 as
//! c_ulong` would sign-extend).

use crate::error::RawError;

/// The `_IOC` direction field: neither read nor write.
const DIR_NONE: u32 = 0;
/// The `_IOC` direction field: the kernel **writes** the argument (`_IOR`).
const DIR_READ: u32 = 2;
/// The `_IOC` direction field: the kernel **reads** the argument (`_IOW`).
const DIR_WRITE: u32 = 1;

/// Bit position of the size field in the `_IOC` word.
const NRBITS: u32 = 8;
/// Bit position of the type ("magic") field.
const TYPEBITS: u32 = 8;
/// Width of the size field — the reason [`readwrite`] can refuse.
const SIZEBITS: u32 = 14;

const NRSHIFT: u32 = 0;
const TYPESHIFT: u32 = NRSHIFT + NRBITS;
const SIZESHIFT: u32 = TYPESHIFT + TYPEBITS;
const DIRSHIFT: u32 = SIZESHIFT + SIZEBITS;

/// The largest payload an ioctl request number can describe.
pub const MAX_IOCTL_SIZE: usize = (1usize << SIZEBITS) - 1;

/// ★★★ The byte count a request number tells the driver to copy — `_IOC_SIZE(request)`.
///
/// This lives beside [`encode`] because it is that function read backwards, and a decoder
/// that drifts from its encoder is how a size check stops checking. ⊘ It is **not** a
/// convenience: `chardev_unsafe`'s `ioctl` compares it against the caller's buffer length,
/// because the driver copies exactly this many bytes **in both directions** at the address it
/// is given (`ogkm-580: kernel-open/nvidia/nv.c:2404` `arg_size = _IOC_SIZE(cmd)`, `:2445`
/// `NV_COPY_FROM_USER(…, arg_size)`, `:2775` `NV_COPY_TO_USER(…, arg_size)`).
#[must_use]
pub fn declared_size(request: u64) -> usize {
    ((request >> SIZESHIFT) as usize) & MAX_IOCTL_SIZE
}

fn encode(dir: u32, ty: u8, nr: u8, size: usize) -> Result<u64, RawError> {
    if size > MAX_IOCTL_SIZE {
        return Err(RawError::TooLargeForHost { value: size as u64 });
    }
    let word = (dir << DIRSHIFT)
        | ((size as u32) << SIZESHIFT)
        | (u32::from(ty) << TYPESHIFT)
        | (u32::from(nr) << NRSHIFT);
    Ok(u64::from(word))
}

/// `_IOWR(ty, nr, size)` — the shape **every** NVIDIA frontend escape uses.
///
/// The driver's own numbers are `_IOWR('F', NV_ESC_*, sizeof(params))`; the parameter
/// struct is read *and* written in place, which is why there is no `_IOW`-only door here.
///
/// # Errors
/// [`RawError::TooLargeForHost`] if `size` exceeds [`MAX_IOCTL_SIZE`] — refused rather
/// than truncated, because a truncated size encodes a *valid but different* request.
pub fn readwrite(ty: u8, nr: u8, size: usize) -> Result<u64, RawError> {
    encode(DIR_READ | DIR_WRITE, ty, nr, size)
}

/// `_IOR(ty, nr, size)` — the kernel writes the argument, the caller supplies only space.
///
/// # Errors
/// [`RawError::TooLargeForHost`], as [`readwrite`].
pub fn read(ty: u8, nr: u8, size: usize) -> Result<u64, RawError> {
    encode(DIR_READ, ty, nr, size)
}

/// `_IOW(ty, nr, size)` — the kernel reads the argument and writes nothing back.
///
/// # Errors
/// [`RawError::TooLargeForHost`], as [`readwrite`].
pub fn write(ty: u8, nr: u8, size: usize) -> Result<u64, RawError> {
    encode(DIR_WRITE, ty, nr, size)
}

/// `_IO(ty, nr)` — no argument at all.
#[must_use]
pub fn none(ty: u8, nr: u8) -> u64 {
    // `size == 0` can never exceed the field, so the `Result` would be a lie.
    encode(DIR_NONE, ty, nr, 0).expect("size 0 always fits the _IOC size field")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned against the **kernel's own** macro expansion, computed by hand from
    /// `include/uapi/asm-generic/ioctl.h`, not against this file's arithmetic — a test
    /// that re-runs the implementation proves only that it is deterministic.
    #[test]
    fn nvidia_escapes_encode_to_the_numbers_the_driver_registers() {
        // NV_ESC_RM_CONTROL = 0x2A, NVOS54_PARAMETERS = 32 bytes, magic 'F' = 0x46.
        // _IOWR('F', 0x2A, 32) = (3 << 30) | (32 << 16) | (0x46 << 8) | 0x2A
        assert_eq!(
            readwrite(b'F', 0x2A, 32).expect("32 fits"),
            0xC020_462A,
            "NV_ESC_RM_CONTROL"
        );
        // NV_ESC_CHECK_VERSION_STR = 200 + 10 = 0xD2, nv_ioctl_rm_api_version_t = 72.
        assert_eq!(
            readwrite(b'F', 0xD2, 72).expect("72 fits"),
            0xC048_46D2,
            "NV_ESC_CHECK_VERSION_STR"
        );
    }

    #[test]
    fn the_direction_bits_are_distinct_and_in_the_top_two_bits() {
        assert_eq!(read(b'F', 1, 4).expect("fits") >> 30, 2);
        assert_eq!(write(b'F', 1, 4).expect("fits") >> 30, 1);
        assert_eq!(readwrite(b'F', 1, 4).expect("fits") >> 30, 3);
        assert_eq!(none(b'F', 1) >> 30, 0);
    }

    /// The refusal, and the exact variant — never `is_err()` (testing doctrine).
    #[test]
    fn a_payload_too_large_for_the_size_field_is_refused_not_truncated() {
        let over = MAX_IOCTL_SIZE + 1;
        assert_eq!(
            readwrite(b'F', 0x2A, over),
            Err(RawError::TooLargeForHost { value: over as u64 }),
        );
        // …and the boundary itself is accepted, so the refusal is not off by one.
        assert!(readwrite(b'F', 0x2A, MAX_IOCTL_SIZE).is_ok());
    }

    /// ★ Non-vacuity for the refusal: had it truncated instead, THIS is the collision it
    /// would have caused — a request number identical to a legitimate smaller ioctl.
    #[test]
    fn truncation_would_have_aliased_a_different_legitimate_request() {
        let honest = readwrite(b'F', 0x2A, 32).expect("32 fits");
        let truncated_size = (1usize << SIZEBITS) + 32;
        assert_eq!(
            truncated_size & MAX_IOCTL_SIZE,
            32,
            "the aliasing premise: the low 14 bits of the oversized value ARE 32"
        );
        assert_eq!(
            readwrite(b'F', 0x2A, truncated_size),
            Err(RawError::TooLargeForHost {
                value: truncated_size as u64
            }),
            "and it is refused rather than encoded as {honest:#x}"
        );
    }
}
