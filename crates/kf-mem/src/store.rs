//! The **one object** — GPGA — as a bounds-checked range. `THE_TRANSLATED_PLANE.md` §18.
//!
//! ⊘⊘⊘ `[w825]` This used to be a bump allocator (`Store::carve`). Under §18 that is the wrong
//! shape: the guest's **own** RM heap chooses offsets inside its framebuffer, and we never carve
//! the guest's object. What we need is only to know the object exists, how long it is, and that
//! an offset the guest names lies inside it. ⇒ `{token, len}` plus a bounds check. No cursor.
//!
//! ★ Our *own* buffers (Translated twin rings, staging) come from **our own** objects, never from
//! this one.

use crate::addr::{HostToken, StoreOffset};

/// Why a range of the store was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// The range `[offset, offset+len)` is not wholly inside the object. Refused by name.
    OutOfObject {
        /// First byte of the refused range.
        offset: u64,
        /// Length of the refused range.
        len: u64,
        /// Length of the object.
        object: u64,
    },
    /// A range of zero bytes.
    ZeroLength,
}

impl StoreRefusal {
    /// The refusal's stable name, as logged.
    pub fn name(&self) -> &'static str {
        match self {
            StoreRefusal::OutOfObject { .. } => "out_of_object",
            StoreRefusal::ZeroLength => "store_zero_length",
        }
    }
}

/// The guest's framebuffer: one RM object. ⊘ A name and a length — never an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    token: HostToken,
    len: u64,
}

impl Store {
    /// A store of `len` bytes, named by the host token of its one object.
    pub fn new(token: HostToken, len: u64) -> Store {
        Store { token, len }
    }
    /// The host token of the object.
    pub fn token(&self) -> HostToken {
        self.token
    }
    /// The object's length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }
    /// Is the object zero bytes long?
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Is `[offset, offset+len)` inside the object? ⊘ `checked_add`, because a guest-named
    /// offset near `u64::MAX` must not wrap into range.
    pub fn slice(&self, offset: u64, len: u64) -> Result<StoreOffset, StoreRefusal> {
        if len == 0 {
            return Err(StoreRefusal::ZeroLength);
        }
        match offset.checked_add(len) {
            Some(end) if end <= self.len => Ok(StoreOffset(offset)),
            _ => Err(StoreRefusal::OutOfObject {
                offset,
                len,
                object: self.len,
            }),
        }
    }
}
