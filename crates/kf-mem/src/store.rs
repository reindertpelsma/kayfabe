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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// The range `[offset, offset+len)` is not wholly inside the object. Refused by name.
    OutOfObject { offset: u64, len: u64, object: u64 },
    ZeroLength,
}

impl StoreRefusal {
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
    pub fn new(token: HostToken, len: u64) -> Store {
        Store { token, len }
    }
    pub fn token(&self) -> HostToken {
        self.token
    }
    pub fn len(&self) -> u64 {
        self.len
    }
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
            _ => Err(StoreRefusal::OutOfObject { offset, len, object: self.len }),
        }
    }
}
