//! ★ Context-DMA resolution through the guest's display **instance memory** — the lookup the display
//! engine's front end performs on every `SET_CONTEXT_DMA_*` handle (`docs/design/V3_DISPLAY.md` §4.4).
//!
//! The guest's CPU-RM writes both halves itself and never RPCs them
//! (`ogkm-580: src/nvidia/src/kernel/gpu/disp/inst_mem/disp_inst_mem.c`, `arch/v03/disp_inst_mem_0300.c`):
//! - a **hash table** at the base of instance memory (`NV_UDISP_HASH_BASE..LIMIT`, 8 KiB = 1024
//!   entries of two words: the object handle, then `CLIENT_ID 13:0 | INSTANCE 24:14 | CHN 31:25`),
//!   filled by linear probing from `instmemHashFunc_v03_00` of (client, handle, channel number);
//! - **context-DMA objects** after it (`NV_UDISP_OBJ_MEM_BASE..LIMIT`), 32-byte aligned, each five
//!   words: `TARGET_NODE 1:0 | ACCESS 2:2 | KIND 20:20`, then base and limit `>> 8` as lo/hi pairs.
//!
//! ⊘ Everything here is a pure function of an instance-memory IMAGE the caller read (bounded to the
//! size the guest declared, `WRITE_INST_MEM`) — no pointer is chased outside it, every probe is
//! bounded by the table's entry count, and a miss is a named `None`, never a guess.
//! ⚠ Removal leaves holes (RM clears an entry to handle 0 / instance 0 and may later re-insert past
//! one), so a lookup probes the WHOLE table from the hash rather than stopping at the first hole.

use crate::regs::Regs;

/// Where a context DMA's bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Video memory: an offset into the guest's framebuffer (the store).
    Vidmem,
    /// System memory: a guest physical (bus) address.
    Sysmem,
}

/// A resolved context DMA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxDma {
    /// Aperture.
    pub target: Target,
    /// Base address (256-byte aligned).
    pub base: u64,
    /// Limit, inclusive (the last byte + 1 is `limit + 1`, 256-byte granular).
    pub limit: u64,
    /// Block-linear kind (else pitch).
    pub block_linear: bool,
    /// Writable.
    pub writable: bool,
}

impl CtxDma {
    /// ★ The address of `[off, off+len)` inside this context DMA, or `None` when any byte of it lies
    /// past the limit (or the arithmetic overflows) — the bound every engine access is checked against.
    #[must_use]
    pub fn span(&self, off: u64, len: u64) -> Option<u64> {
        let end = off.checked_add(len)?;
        let size = self.limit.checked_sub(self.base)?.checked_add(1)?;
        (len > 0 && end <= size).then(|| self.base + off)
    }
}

/// Why a lookup failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// Handle 0: the context DMA was cleared (a legal "none" for notifiers).
    NullHandle,
    /// The image is smaller than the hash table (instance memory not stated, or short).
    NoTable,
    /// No entry for (client, handle, channel).
    NotBound,
    /// The entry names an instance outside the object area or the image.
    BadInstance(u32),
    /// The object's target node is not one the engine can reach (0 = invalid).
    BadTarget(u32),
}

/// The derived layout constants the lookup needs, from the family's register vocabulary.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    hash_base: u64,
    hash_bytes: u64,
    obj_base: u64,
    obj_limit: u64,
    client_id: (u8, u8),
    instance: (u8, u8),
    chn: (u8, u8),
    target: (u8, u8),
    access: (u8, u8),
    kind: (u8, u8),
    base_lo: (u8, u8),
    base_hi: (u8, u8),
    limit_lo: (u8, u8),
    limit_hi: (u8, u8),
    nvm: u32,
    pci: u32,
    pci_coherent: u32,
    kind_bl: u32,
    rw: u32,
}

/// The word index and in-word field of a structure bit range `(w*32+hi):(w*32+lo)`.
fn word_field(r: &Regs, name: &str) -> Option<(usize, (u8, u8))> {
    let (hi, lo) = r.f(name)?;
    let w = lo / 32;
    (hi / 32 == w).then(|| (w as usize, ((hi - 32 * w) as u8, (lo - 32 * w) as u8)))
}

impl Layout {
    /// The layout from the derived registers (`NV_UDISP_HASH_*`, `NV_DMA_*`), or `None` when any is
    /// missing or shaped unexpectedly (a refusal, never a default).
    #[must_use]
    pub fn from_regs(r: &Regs) -> Option<Layout> {
        let (cw, client_id) = word_field(r, "NV_UDISP_HASH_TBL_CLIENT_ID")?;
        let (iw, instance) = word_field(r, "NV_UDISP_HASH_TBL_INSTANCE")?;
        let (hw, chn) = word_field(r, "NV_UDISP_HASH_TBL_CHN")?;
        let (tw, target) = word_field(r, "NV_DMA_TARGET_NODE")?;
        let (aw, access) = word_field(r, "NV_DMA_ACCESS")?;
        let (kw, kind) = word_field(r, "NV_DMA_KIND")?;
        let (blw, base_lo) = word_field(r, "NV_DMA_ADDRESS_BASE_LO")?;
        let (bhw, base_hi) = word_field(r, "NV_DMA_ADDRESS_BASE_HI")?;
        let (llw, limit_lo) = word_field(r, "NV_DMA_ADDRESS_LIMIT_LO")?;
        let (lhw, limit_hi) = word_field(r, "NV_DMA_ADDRESS_LIMIT_HI")?;
        // the shapes the lookup below reads: entry word 1 holds all three hash fields; the object's
        // flags live in word 0 and the four address words are 1..4
        if (cw, iw, hw, tw, aw, kw, blw, bhw, llw, lhw) != (1, 1, 1, 0, 0, 0, 1, 2, 3, 4) {
            return None;
        }
        let hash_base = r.v("NV_UDISP_HASH_BASE")?;
        let hash_limit = r.v("NV_UDISP_HASH_LIMIT")?;
        Some(Layout {
            hash_base,
            hash_bytes: hash_limit.checked_sub(hash_base)?.checked_add(1)?,
            obj_base: r.v("NV_UDISP_OBJ_MEM_BASE")?,
            obj_limit: r.v("NV_UDISP_OBJ_MEM_LIMIT")?,
            client_id,
            instance,
            chn,
            target,
            access,
            kind,
            base_lo,
            base_hi,
            limit_lo,
            limit_hi,
            nvm: r.v32("NV_DMA_TARGET_NODE_PHYSICAL_NVM")?,
            pci: r.v32("NV_DMA_TARGET_NODE_PHYSICAL_PCI")?,
            pci_coherent: r.v32("NV_DMA_TARGET_NODE_PHYSICAL_PCI_COHERENT")?,
            kind_bl: r.v32("NV_DMA_KIND_BLOCKLINEAR")?,
            rw: r.v32("NV_DMA_ACCESS_READ_AND_WRITE")?,
        })
    }

    /// The hash table's size in bytes (what a caller must read at least).
    #[must_use]
    pub fn table_bytes(&self) -> u64 {
        self.hash_base + self.hash_bytes
    }

    /// The whole instance memory this layout addresses (hash table and object area).
    #[must_use]
    pub fn inst_bytes(&self) -> u64 {
        self.obj_limit + 1
    }

    /// `instmemHashFunc_v03_00` (`disp_inst_mem_0300.c:88-116`), masked to the table.
    #[must_use]
    pub fn hash(&self, client: u32, handle: u32, chn: u32) -> u32 {
        let h = (handle & 0x3FF)
            ^ ((handle >> 10) & 0x3FF)
            ^ ((handle >> 20) & 0x3FF)
            ^ (((client & 0xFF) << 2) | (handle >> 30))
            ^ (((chn & 0xF) << 6) | ((client >> 8) & 0x3F))
            ^ ((chn >> 4) & 0x7);
        h & (self.entries() - 1)
    }

    fn entries(&self) -> u32 {
        u32::try_from(self.hash_bytes / 8).unwrap_or(0).max(1)
    }

    /// ★ Resolve `handle` bound to channel number `chn` by `client` in the instance-memory `image`.
    ///
    /// # Errors
    /// [`Miss`], by name.
    pub fn resolve(&self, image: &[u8], client: u32, handle: u32, chn: u32) -> Result<CtxDma, Miss> {
        if handle == 0 {
            return Err(Miss::NullHandle);
        }
        let n = self.entries();
        if (image.len() as u64) < self.table_bytes() || !n.is_power_of_two() {
            return Err(Miss::NoTable);
        }
        let word = |off: u64| -> Option<u32> {
            let o = usize::try_from(off).ok()?;
            Some(u32::from_le_bytes(image.get(o..o + 4)?.try_into().ok()?))
        };
        let client_mask = (1u32 << (self.client_id.0 - self.client_id.1 + 1)) - 1;
        let chn_mask = (1u32 << (self.chn.0 - self.chn.1 + 1)) - 1;
        let start = self.hash(client, handle, chn);
        for i in 0..n {
            let e = u64::from((start + i) & (n - 1));
            let at = self.hash_base + e * 8;
            let (Some(obj), Some(ctx)) = (word(at), word(at + 4)) else { return Err(Miss::NoTable) };
            if obj != handle {
                continue;
            }
            let inst = crate::class::get(ctx, self.instance);
            if inst == 0
                || crate::class::get(ctx, self.client_id) != client & client_mask
                || crate::class::get(ctx, self.chn) != chn & chn_mask
            {
                continue;
            }
            return self.object(image, inst);
        }
        Err(Miss::NotBound)
    }

    /// The context DMA object at `instance` (32-byte units, `disp_inst_mem.c:640-650`).
    fn object(&self, image: &[u8], instance: u32) -> Result<CtxDma, Miss> {
        let at = u64::from(instance) << 5;
        if at <= self.hash_base + self.hash_bytes - 1 || at < self.obj_base || at + 20 > self.obj_limit + 1 {
            return Err(Miss::BadInstance(instance));
        }
        let w = |i: u64| -> Result<u32, Miss> {
            let o = usize::try_from(at + 4 * i).map_err(|_| Miss::BadInstance(instance))?;
            image
                .get(o..o + 4)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or(Miss::BadInstance(instance))
        };
        let (w0, w1, w2, w3, w4) = (w(0)?, w(1)?, w(2)?, w(3)?, w(4)?);
        let node = crate::class::get(w0, self.target);
        let target = if node == self.nvm {
            Target::Vidmem
        } else if node == self.pci || node == self.pci_coherent {
            Target::Sysmem
        } else {
            return Err(Miss::BadTarget(node));
        };
        let g = crate::class::get;
        let base = (u64::from(g(w2, self.base_hi)) << 32 | u64::from(g(w1, self.base_lo))) << 8;
        let limit = ((u64::from(g(w4, self.limit_hi)) << 32 | u64::from(g(w3, self.limit_lo))) << 8) | 0xFF;
        if limit < base {
            return Err(Miss::BadInstance(instance));
        }
        Ok(CtxDma {
            target,
            base,
            limit,
            block_linear: g(w0, self.kind) == self.kind_bl,
            writable: g(w0, self.access) == self.rw,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn layout() -> Layout {
        Layout::from_regs(&Regs::for_ip("580.159.04", 0x0401_0000).unwrap()).expect("derived layout")
    }

    /// Write what `instmemCommitContextDma_v03_00` + `_instmemAddHashEntry` write, into `img`.
    pub(crate) fn bind(img: &mut [u8], l: &Layout, client: u32, handle: u32, chn: u32, inst32: u32, node: u32, base: u64, limit: u64) {
        let mut e = l.hash(client, handle, chn) as usize;
        loop {
            let at = e * 8;
            if u32::from_le_bytes(img[at..at + 4].try_into().unwrap()) == 0 {
                break;
            }
            e = (e + 1) % 1024;
        }
        let ctx = (client & 0x3FFF) | (inst32 << 14) | (chn << 25);
        img[e * 8..e * 8 + 4].copy_from_slice(&handle.to_le_bytes());
        img[e * 8 + 4..e * 8 + 8].copy_from_slice(&ctx.to_le_bytes());
        let o = (inst32 as usize) << 5;
        let words = [node | (1 << 2), (base >> 8) as u32, (base >> 40) as u32, (limit >> 8) as u32, (limit >> 40) as u32];
        for (i, w) in words.iter().enumerate() {
            img[o + 4 * i..o + 4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
    }

    /// ★ A context DMA bound the way the guest's RM binds one resolves to its base and limit; a
    /// second handle colliding on the same hash is found by probing; the channel and client are part
    /// of the key.
    #[test]
    fn bound_ctxdmas_resolve_and_collisions_probe() {
        let l = layout();
        assert_eq!(l.table_bytes(), 0x2000);
        assert_eq!(l.inst_bytes(), 0x1_0000);
        let mut img = vec![0u8; 0x1_0000];
        let c = 0xc1d0_0015;
        bind(&mut img, &l, c, 0xcaf0_0010, 0, 0x100, 2, 0x1_2345_6000, 0x1_2345_6fff);
        // a handle chosen to collide with the first on channel 0
        let other = (0..0xFFFFu32).map(|x| 0xbee0_0000 | x).find(|h| l.hash(c, *h, 0) == l.hash(c, 0xcaf0_0010, 0)).unwrap();
        bind(&mut img, &l, c, other, 0, 0x101, 1, 0x4000_0000, 0x4000_ffff);
        let a = l.resolve(&img, c, 0xcaf0_0010, 0).unwrap();
        assert_eq!((a.target, a.base, a.limit, a.writable), (Target::Sysmem, 0x1_2345_6000, 0x1_2345_6fff, true));
        let b = l.resolve(&img, c, other, 0).unwrap();
        assert_eq!((b.target, b.base), (Target::Vidmem, 0x4000_0000));
        assert_eq!(l.resolve(&img, c, 0xcaf0_0010, 1), Err(Miss::NotBound), "bound to channel 0 only");
        assert_eq!(l.resolve(&img, c + 1, 0xcaf0_0010, 0), Err(Miss::NotBound), "another client");
        assert_eq!(l.resolve(&img, c, 0, 0), Err(Miss::NullHandle));
        assert_eq!(a.span(0x10, 0x10), Some(0x1_2345_6010));
        assert_eq!(a.span(0xff0, 0x11), None, "one byte past the limit");
        assert_eq!(a.span(u64::MAX, 2), None, "overflow");
    }

    /// ⊘ Hostile guest: an entry pointing into the hash table, past the image, or with an invalid
    /// target node is refused by name; a short image is `NoTable`; a full table of non-matching
    /// entries terminates (bounded by the entry count).
    #[test]
    fn hostile_instance_memory_is_refused_bounded() {
        let l = layout();
        let c = 0xc1d0_0001;
        let mut img = vec![0u8; 0x1_0000];
        bind(&mut img, &l, c, 0x11, 0, 0x10, 2, 0, 0xfff); // instance 0x10 << 5 = 0x200: inside the hash table
        assert_eq!(l.resolve(&img, c, 0x11, 0), Err(Miss::BadInstance(0x10)));
        let mut img = vec![0u8; 0x1_0000];
        bind(&mut img, &l, c, 0x12, 0, 0x7ff, 0, 0, 0xfff); // node 0
        assert_eq!(l.resolve(&img, c, 0x12, 0), Err(Miss::BadTarget(0)));
        assert_eq!(l.resolve(&img[..0x1000], c, 0x12, 0), Err(Miss::NoTable));
        let mut full = vec![0u8; 0x1_0000];
        for e in 0..1024usize {
            full[e * 8..e * 8 + 4].copy_from_slice(&0xdead_beefu32.to_le_bytes());
        }
        assert_eq!(l.resolve(&full, c, 0x13, 0), Err(Miss::NotBound));
        // an instance whose object would run past a short image
        let mut img = vec![0u8; 0x2100];
        let e = l.hash(c, 0x14, 0) as usize;
        img[e * 8..e * 8 + 4].copy_from_slice(&0x14u32.to_le_bytes());
        img[e * 8 + 4..e * 8 + 8].copy_from_slice(&((c & 0x3FFF) | (0x7ff << 14)).to_le_bytes());
        assert_eq!(l.resolve(&img, c, 0x14, 0), Err(Miss::BadInstance(0x7ff)));
    }
}
