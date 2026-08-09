//! ★ The **work-submission** ABI — the layouts and numbers an RM client needs to put
//! work on a real engine, as opposed to the layouts [`crate::bringup`] needs to exist.
//!
//! [`crate::bringup`] gets an isolate as far as *"a client, a device, an address space,
//! a memory object, a GPU mapping"*. Everything here is the next thing: a **channel**, a
//! ring the channel fetches from, the token that names it to hardware, and the methods a
//! copy engine executes. `kayfabe_isolate_host::rm`'s module docs enumerate exactly this
//! as the machinery its five refused verbs lack — *"a GPFIFO ring and USERD in mapped
//! memory, a channel group and a context share, a work-submit token read back by a
//! control, a doorbell mapping rather than an ioctl"*.
//!
//! ## ★★ Provenance, and why this is a separate module from the generated mirrors
//!
//! Everything here is read off the **bench's own driver**, `ogkm-580: 580.159.04`, and
//! the central struct *cannot* come from the generator: `crate::generated::classes`'
//! own docs record that `NV_CHANNEL_ALLOC_PARAMS` diverges between the two vendored
//! trees — at `ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` there
//! is an `hHandleVASpace` at +32 that does **not** exist at
//! `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342`, so every field
//! from +32 onward differs by four bytes between them. A generated 610 mirror would put
//! `engineType` at +132 for a driver that reads it at +128 — i.e. it would route the
//! channel to a **different runlist**, which is the C's proven `engineType = 0` bug
//! class (`dma_copy_class_alloc_params`, seam audit GR-1) arrived at by a different
//! road.
//!
//! So [`ChannelAllocParams`] is an **offset-addressed encoder** rather than a
//! `#[repr(C)]` mirror, for the same reason `crate::versions::CHANNEL_ALLOC_PREFIX` is a
//! prefix decoder: the fields we set are the ones both trees agree on plus the ones 580
//! places where 580 places them, and the ~30 reserved members in the tail are zero on
//! the wire and are never read back. Mirroring them would be breadth, and a mirror that
//! is right for the wrong tree is worse than no mirror.
//!
//! ## What is deliberately NOT here
//!
//! No policy. This module says where `engineType` lives and what value means "the first
//! copy engine"; it does not say which engine a channel should be on. That is
//! `kayfabe_isolate::RmBackend::alloc_channel`'s `engine` argument, declared by the
//! caller because an engine-blind channel alloc is the bug class named above.

use crate::wire::{AbiError, u32_at};

// =====================================================================================
// Escapes this module adds to the frontend set kayfabe-abi::bringup opened
// =====================================================================================

/// `NV_ESC_RM_MAP_MEMORY` —
/// `ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv_escape.h:42`.
///
/// ★★ **This escape is only half of a mapping.** It validates the request and registers
/// an mmap *context* against a descriptor the caller supplies
/// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:529-534`); the mapping itself
/// happens when the caller then `mmap`s **that descriptor**. Three consequences that are
/// invisible from the struct and each cost a real failure to find:
///
/// 1. The escape is `NV_CTL_DEVICE_ONLY` (`escape.c:521`) — it is issued on the control
///    node **regardless** of which node the resulting `mmap` uses.
/// 2. The descriptor in [`Nvos33ParametersWithFd::fd`] must be of the **right kind**:
///    RM picks the device node's state for an address inside a BAR and the control
///    node's for system memory (`ogkm-580: .../osapi.c:2270-2279`), and
///    `nv_get_file_private(fd, NV_IS_CTL_DEVICE(nv), …)` then refuses a descriptor of
///    the other kind outright (`ogkm-580: kernel-open/nvidia/nv-usermap.c:45-47`).
/// 3. The context is **one-shot per descriptor**: a second registration on a descriptor
///    that already has a live one is `NV_ERR_STATE_IN_USE`
///    (`ogkm-580: kernel-open/nvidia/nv-usermap.c:53-57`). Every mapping needs its own
///    freshly opened node.
pub const NV_ESC_RM_MAP_MEMORY: u8 = 0x4E;

/// `NV_ESC_RM_UNMAP_MEMORY` —
/// `ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv_escape.h:43`.
pub const NV_ESC_RM_UNMAP_MEMORY: u8 = 0x4F;

/// The `mmap` file offset the driver accepts for a mapping registered by
/// [`NV_ESC_RM_MAP_MEMORY`] — **zero, and only zero**.
///
/// `nvidia_mmap_helper` refuses any non-zero `vm_pgoff` with `EINVAL`
/// (`ogkm-580: kernel-open/nvidia/nv-mmap.c:533-536`), and the length must equal the
/// registered context's size exactly (`:562-565`). The offset *within* the object is the
/// one given to the escape, not to `mmap` — which is the opposite of the usual device
/// convention and reads as a bug at every call site that does not say so.
pub const MMAP_FILE_OFFSET: u64 = 0;

// =====================================================================================
// NVOS33_PARAMETERS + fd — the CPU-mapping request
// =====================================================================================

/// `nv_ioctl_nvos33_parameters_with_fd` —
/// `ogkm-580: src/nvidia/arch/nvalloc/unix/include/nv-unix-nvos-params-wrappers.h:42-46`,
/// wrapping `NVOS33_PARAMETERS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:1844-1854`).
///
/// Same shape of thing as [`crate::bringup::Nvos02ParametersWithFd`]: the SDK struct plus
/// an `int fd` the Unix frontend appends, so `sizeof` is the wrapper's and not the SDK's.
///
/// ★ `p_linear_address` is `[OUT]` — RM writes the *kernel's* notion of the address, and
/// it is **not** something userspace may dereference or is even expected to use. It is
/// carried because it is what the subsequent `mmap` context was registered against, and
/// because `NV_ESC_RM_UNMAP_MEMORY` names it. Nothing outside the raw adapter ever sees
/// it, which is why this field is not a host address in the sense
/// `kayfabe_linux_raw`'s §4.2.1 refusal is about: it is an opaque cookie RM minted.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nvos33ParametersWithFd {
    /// `NvHandle hClient` @ +0.
    pub h_client: u32,
    /// `NvHandle hDevice` @ +4 — device **or subdevice** handle.
    pub h_device: u32,
    /// `NvHandle hMemory` @ +8 — the object to map. For USERD this is the **channel**
    /// handle: RM maps a channel's USERD when the channel object itself is the map
    /// target (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:1277-1324`).
    pub h_memory: u32,
    // +12: four bytes of padding before the 8-aligned `offset`.
    /// `NvU64 offset` @ +16 — the offset **within the object**, not within the mapping.
    pub offset: u64,
    /// `NvU64 length` @ +24.
    pub length: u64,
    /// `NvP64 pLinearAddress` @ +32 — `[OUT]`, an opaque cookie (see the type docs).
    pub p_linear_address: u64,
    /// `NvU32 status` @ +40 — `[OUT]`.
    pub status: u32,
    /// `NvU32 flags` @ +44. ★ The caching bits here are **ignored**: the frontend
    /// overwrites the field with `_CACHING_TYPE_DEFAULT` before RM sees it
    /// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/escape.c:523-524`, comment
    /// *"Don't allow userspace to override the caching type"*). Cacheability is
    /// decided by what the pages ARE — exactly the finding `kayfabe_linux_raw::cache`
    /// is built around, here confirmed by the driver refusing to be told.
    pub flags: u32,
    /// `int fd` @ +48 — the descriptor the mmap context is registered against.
    pub fd: i32,
    // +52: four bytes of tail padding to the wrapper's 8-byte alignment.
}

impl Default for Nvos33ParametersWithFd {
    /// `fd = -1`, everything else zero. Same reasoning as
    /// [`crate::bringup::Nvos02ParametersWithFd`]: zero is a *valid descriptor number*,
    /// so a defaulted struct must not name one.
    fn default() -> Self {
        Nvos33ParametersWithFd {
            h_client: 0,
            h_device: 0,
            h_memory: 0,
            offset: 0,
            length: 0,
            p_linear_address: 0,
            status: 0,
            flags: 0,
            fd: -1,
        }
    }
}

impl Nvos33ParametersWithFd {
    /// The C typedef name.
    pub const C_NAME: &'static str = "nv_ioctl_nvos33_parameters_with_fd";
    /// `sizeof`.
    pub const SIZE: usize = 56;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.h_client.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            4,
            &self.h_device.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            8,
            &self.h_memory.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            16,
            &self.offset.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            24,
            &self.length.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            32,
            &self.p_linear_address.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            40,
            &self.status.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            44,
            &self.flags.to_le_bytes(),
        )?;
        put(bytes, Self::C_NAME, Self::SIZE, 48, &self.fd.to_le_bytes())
    }

    /// Decode a little-endian image.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn decode(bytes: &[u8]) -> Result<Self, AbiError> {
        Ok(Nvos33ParametersWithFd {
            h_client: u32_at(bytes, 0)?,
            h_device: u32_at(bytes, 4)?,
            h_memory: u32_at(bytes, 8)?,
            offset: crate::wire::u64_at(bytes, 16)?,
            length: crate::wire::u64_at(bytes, 24)?,
            p_linear_address: crate::wire::u64_at(bytes, 32)?,
            status: u32_at(bytes, 40)?,
            flags: u32_at(bytes, 44)?,
            #[expect(
                clippy::cast_possible_wrap,
                reason = "`int fd`: the wire field IS signed, and -1 is the value that means \
                          'no descriptor'. Reading it as unsigned would turn that into 2^32-1."
            )]
            fd: u32_at(bytes, 48)? as i32,
        })
    }
}

// =====================================================================================
// NV_CHANNEL_ALLOC_PARAMS — the 580 shape, by offset
// =====================================================================================

/// `NV_CHANNEL_ALLOC_PARAMS` at `ogkm-580: 580.159.04` — an **offset-addressed encoder**,
/// not a `#[repr(C)]` mirror. The module docs say why; the short version is that the 610
/// tree has one extra handle at +32 and every field after it moves.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-342`. The layout, with
/// `NV_MAX_SUBDEVICES = 8` (`ogkm-580: src/common/sdk/nvidia/inc/nvlimits.h:42`) and
/// `NV_MEMORY_DESC_PARAMS` = 24 bytes (`alloc_channel.h:37-42`):
///
/// ```text
///   +0    hObjectError        +128  engineType        +240  hPhysChannelGroup
///   +4    hObjectBuffer       +132  cid               +244  internalFlags
///   +8    gpFifoOffset (u64)  +136  subDeviceId       +248  errorNotifierMem
///   +16   gpFifoEntries       +140  hObjectEccError   +272  eccErrorNotifierMem
///   +20   flags               +144  instanceMem       +296  ProcessID
///   +24   hContextShare       +168  userdMem          +300  SubProcessID
///   +28   hVASpace            +192  ramfcMem          +304  encryptIv[3]
///   +32   hUserdMemory[8]     +216  mthdbufMem        +316  decryptIv[3]
///   +64   userdOffset[8]                              +328  hmacNonce[8]
///                                                     +360  tpcConfigID
///   sizeof = 368 (364 rounded to the 8-byte alignment `NV_DECLARE_ALIGNED` forces)
/// ```
///
/// Everything from +144 on is `// reserved` in the header and is left zero. The fields
/// this struct names are the ones a client must fill for a channel to exist and land on
/// the right runlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelAllocParams {
    /// `NvHandle hObjectError` @ +0 — the error-notifier context DMA. Zero is legal and
    /// means *"do not report channel errors through a notifier"*; the isolate learns
    /// about a wedged channel from the verb that does not complete, not from here.
    pub h_object_error: u32,
    /// `NvU64 gpFifoOffset` @ +8 — the **GPU virtual address** of the GPFIFO ring, in
    /// this channel's own address space. Not a CPU address and not a physical one.
    pub gp_fifo_offset: u64,
    /// `NvU32 gpFifoEntries` @ +16 — the ring's capacity in 8-byte entries, so the ring
    /// is `gp_fifo_entries * 8` bytes long. RM requires a power of two.
    pub gp_fifo_entries: u32,
    /// `NvU32 flags` @ +20 — `NVOS04_FLAGS_*`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:65-287`). Zero is the
    /// ordinary unprivileged channel, which is the only kind an isolate may have.
    pub flags: u32,
    /// `NvHandle hContextShare` @ +24 — a `FERMI_CONTEXT_SHARE_A` under this channel's
    /// TSG, or **zero to inherit the TSG's**. The C's host channel leaves it zero
    /// (`C: src/qemu/nvkvm_gpu_emul.c:9517-9522`), and so does this port: a subcontext
    /// is a *sharing* mechanism between channels, and an isolate's own channel shares
    /// with nothing.
    pub h_context_share: u32,
    /// `NvHandle hVASpace` @ +28 — the `FERMI_VASPACE_A`.
    ///
    /// ★★ **Zero, for a channel under a TSG, and this is not an omission.** The TSG
    /// declares the address space and its channels inherit it; naming one *again* here
    /// is refused by host RM (`C: src/qemu/nvkvm_gpu_emul.c:7049-7052`, *"TSG channels
    /// can't use an explicit vaspace"*). The place a host VAS is declared is
    /// [`crate::generated::classes::NvChannelGroupAllocationParameters::h_va_space`].
    ///
    /// ★ And when it *is* set, it is the **VASpace object**, never the
    /// `NV01_MEMORY_VIRTUAL` range over it — two different handles, and
    /// `kayfabe_isolate::RmBackend::alloc_vaspace` returns the *range*, because that is
    /// what `NV_ESC_RM_MAP_MEMORY_DMA` needs.
    pub h_va_space: u32,
    /// `NvHandle hUserdMemory[0]` @ +32 — **client-allocated USERD**, and this port
    /// supplies one.
    ///
    /// The header says *"ignored if hUserdMemory[0]=0"*, in which case RM allocates
    /// USERD itself
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_channel_gv100.c:80`
    /// takes the client-allocated branch only when the handle is non-zero). ★ We do not
    /// take that option, because the C did not: its proven host channel passes its own
    /// 64 KiB vidmem object (`C: src/qemu/nvkvm_gpu_emul.c:9520`). Owning the object is
    /// what lets us map it with one ordinary [`NV_ESC_RM_MAP_MEMORY`] on a *memory*
    /// handle instead of relying on `kchannelMap`, and USERD's two cursors
    /// ([`USERD_GP_GET`], [`USERD_GP_PUT`]) are where all of this rung's hardware
    /// evidence is read from.
    pub h_userd_memory_0: u32,
    /// `NvU64 userdOffset[0]` @ +64 — the offset of USERD **within**
    /// [`Self::h_userd_memory_0`].
    ///
    /// ★★★ **Zero, and the C paid for that in days.** The host channel's USERD address
    /// is `hUserdMemory[0] + userdOffset[0]`. A guest pools many channels' USERDs into
    /// one object and addresses each by a non-zero offset; substituting a fresh
    /// per-channel object while carrying the guest's offset across makes hardware read
    /// USERD *past* the object while our `GP_PUT` lands at offset 0 — so the GPU sees
    /// `GP_PUT == GP_GET` forever, fetches nothing, and reports **no error at all**
    /// (`C: src/qemu/nvkvm_gpu_emul.c:9291-9299`, the M5.47 root-cause fix). Zero
    /// utilisation and no Xid is the worst failure shape available.
    pub userd_offset_0: u64,
    /// `NvU32 engineType` @ +128 — an `NV2080_ENGINE_TYPE_*`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:282,291,396`).
    /// See [`ENGINE_TYPE_GRAPHICS`] / [`engine_type_copy`].
    ///
    /// ## ★★★ MEASURED: for a channel **inside a TSG**, this field is INERT
    ///
    /// The offset is the whole reason this module is hand-written — 610 puts `engineType`
    /// at +132 — so it was bitten on hardware (RTX 3060 / 580.159.04): the encoder was
    /// changed to write +132 and the engine-type sweep re-run. **Nothing changed.** Every
    /// engine still routed to the same runlist it routed to before.
    ///
    /// The follow-up separated the two candidate fields. Zeroing *this* field while
    /// leaving the group's changed nothing; zeroing the **group's**
    /// ([`crate::generated::classes::NvChannelGroupAllocationParameters::engine_type`])
    /// while leaving this one made every allocation fail with `NV_ERR_INVALID_ARGUMENT`
    /// (0x1F). So the routing decision is the **TSG's**, and a channel in a group inherits
    /// it — exactly like `hVASpace` and `hContextShare` two fields up.
    ///
    /// ★ Two honest consequences, both worth more than the tidy version:
    ///
    /// 1. **The +128 offset is NOT verified by this hardware.** It is read off the 580
    ///    header and asserted by the encoder test, and that is all. A configuration where
    ///    it *is* load-bearing — a channel with no TSG — is the only thing that could
    ///    check it, and this port does not build one.
    /// 2. This field is nonetheless still sent, because the C sends it
    ///    (`C: src/qemu/nvkvm_gpu_emul.c:9520`) and *"port the C, subtract only its named
    ///    bugs"*. Removing it would be a redesign justified by one part's behaviour.
    pub engine_type: u32,
}

impl ChannelAllocParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV_CHANNEL_ALLOC_PARAMS";
    /// `sizeof` at 580.159.04. ★ RM discriminates on `paramsSize`, so this number is
    /// part of the request: sending 364 (the un-rounded sum) is a different, refused
    /// request from sending 368.
    pub const SIZE: usize = 368;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes. Every byte
    /// this struct does not name is left as found — so the caller must supply a
    /// **zeroed** buffer, which is what the reserved tail requires.
    ///
    /// ★★ The length is checked **up front**, and that is not the same check the `put`
    /// calls below make. The last field this encoder writes is at +128 of a 368-byte
    /// struct, so a per-field bounds check accepts any buffer ≥ 132 bytes and encodes
    /// perfectly into it — and the caller then sends `paramsSize = 132`, which RM reads
    /// as a *different request*. A test caught exactly that: the per-field checks passed
    /// on 367 bytes. The reserved tail is part of the request even though nothing writes
    /// it.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let n = Self::C_NAME;
        let s = Self::SIZE;
        if bytes.len() < s {
            return Err(AbiError::Truncated {
                c_name: n,
                need: s,
                got: bytes.len(),
            });
        }
        put(bytes, n, s, 0, &self.h_object_error.to_le_bytes())?;
        put(bytes, n, s, 8, &self.gp_fifo_offset.to_le_bytes())?;
        put(bytes, n, s, 16, &self.gp_fifo_entries.to_le_bytes())?;
        put(bytes, n, s, 20, &self.flags.to_le_bytes())?;
        put(bytes, n, s, 24, &self.h_context_share.to_le_bytes())?;
        put(bytes, n, s, 28, &self.h_va_space.to_le_bytes())?;
        put(bytes, n, s, 32, &self.h_userd_memory_0.to_le_bytes())?;
        put(bytes, n, s, 64, &self.userd_offset_0.to_le_bytes())?;
        put(bytes, n, s, 128, &self.engine_type.to_le_bytes())
    }
}

/// `NV2080_ENGINE_TYPE_GRAPHICS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:282`.
pub const ENGINE_TYPE_GRAPHICS: u32 = 0x0000_0001;

/// `NV2080_ENGINE_TYPE_COPY0` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:291`.
pub const ENGINE_TYPE_COPY0: u32 = 0x0000_0009;

/// `NV2080_ENGINE_TYPE_COPY(i)` for `i < 10` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:396`.
///
/// **Only `i < 10`.** The macro's other arm re-bases on `NV2080_ENGINE_TYPE_COPY10`,
/// which is a different, discontiguous block; this returns `None` there rather than
/// computing a number that names a *different engine class*. A copy engine index past 9
/// on a part that has one is a real request, and it needs the second constant read off
/// the header — not an extrapolation.
#[must_use]
pub const fn engine_type_copy(index: u32) -> Option<u32> {
    if index < 10 {
        Some(ENGINE_TYPE_COPY0 + index)
    } else {
        None
    }
}

/// `NV2080_ENGINE_TYPE_COPY10` — the base of the **second, discontiguous** copy-engine
/// block (`ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:340`).
pub const ENGINE_TYPE_COPY10: u32 = 0x0000_0034;

/// The exact inverse of `NV2080_ENGINE_TYPE_COPY(i)` — *"which copy engine is this?"*
///
/// `NV2080_ENGINE_TYPE_IS_COPY` then `NV2080_ENGINE_TYPE_COPY_IDX`, transcribed as one
/// function (`ogkm-580: cl2080_notification.h:396-400`): `COPY0..COPY9` are `0x09..=0x12`
/// and `COPY10..COPY19` are `0x34..=0x3d`, and **the gap between them is real** —
/// `0x13` in this space is `NVDEC0`, not `COPY10`.
///
/// ⊘ `None` means *"not a copy engine"* and never *"copy engine 0"*. That distinction is
/// the whole reason this returns an `Option`: a caller reporting which CE the guest named
/// must be able to say "it named something else", and a zero would read as CE0 — which is
/// one of the two indices whose non-stall vector this chip publishes as `INVALID`, i.e.
/// precisely the answer a wrong default would fake.
///
/// ⚠ Both blocks are covered on purpose even though this port's GA106 has five CEs: an
/// inverse that silently stops at `COPY9` would answer `None` for a real engine on a
/// later part, and that is the too-strict half of `mock_fidelity_both_directions`.
#[must_use]
pub const fn copy_index_of_engine_type(engine_type: u32) -> Option<u32> {
    if engine_type >= ENGINE_TYPE_COPY0 && engine_type <= ENGINE_TYPE_COPY0 + 9 {
        Some(engine_type - ENGINE_TYPE_COPY0)
    } else if engine_type >= ENGINE_TYPE_COPY10 && engine_type <= ENGINE_TYPE_COPY10 + 9 {
        Some(engine_type - ENGINE_TYPE_COPY10 + 10)
    } else {
        None
    }
}

// =====================================================================================
// The two channel controls
// =====================================================================================

/// ★★ `NVA06C_CTRL_CMD_BIND` = `0xa06c0102`, issued **on the TSG**, params a single
/// `NvU32 engineType` (4 bytes).
///
/// **This must happen before [`NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN`], and the
/// failure if it does not is a status that names nothing about binding.** RM generates a
/// work-submit token from the channel's runlist assignment, and a channel that has not
/// been bound has no runlist yet — so the token control answers
/// `NV_ERR_INVALID_STATE` (0x40) (`C: src/qemu/nvkvm_gpu_emul.c:9568-9572`, and the same
/// diagnosis at `:4080-4088`). The C found this the expensive way; it is written down
/// here so the ordering is a property of the code rather than of whoever remembers.
///
/// The command number is Kepler-era (`a06c` = `KEPLER_CHANNEL_GROUP_A`) and is the live
/// one for every later TSG: the class inherits the interface rather than renumbering it.
pub const NVA06C_CTRL_CMD_BIND: u32 = 0xa06c_0102;

/// `sizeof(NVA06C_CTRL_BIND_PARAMS)` — a single `NvU32 engineType`.
pub const BIND_PARAMS_SIZE: usize = 4;

/// `NVA06C_CTRL_CMD_GPFIFO_SCHEDULE` = `0xa06c0101`, issued **on the TSG**.
///
/// ★ There are two schedule commands and they are not interchangeable: this one takes a
/// channel *group*, and [`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`] takes a single channel. A
/// channel that lives in a TSG is scheduled by scheduling its group — which is what the
/// C's proven host channel does (`C: src/qemu/nvkvm_gpu_emul.c:9577`), and what this
/// port does. Both take [`GpfifoScheduleParams`].
pub const NVA06C_CTRL_CMD_GPFIFO_SCHEDULE: u32 = 0xa06c_0101;

/// `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:66`.
///
/// Issued **on the channel object**, for a channel that is not in a TSG.
///
/// ★★★ **This is the command the guest sends us, and it is NOT the one we send the host.**
/// The two directions were conflated in this module's earlier text ("present for
/// completeness and *not* what this port uses"), and that sentence was wrong about the
/// **guest** direction. Established from the driver's own source rather than assumed
/// (task #177, method step 1):
///
/// - `_memmgrMemUtilsScrubInitScheduleChannel` issues **this** id — `0xa06f0103`, on
///   `pChannel->channelId`, a bare channel with no TSG — for the global CeUtils scrubber
///   (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/mem_utils.c:1973-1989`). The TSG form
///   [`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`] is **not** on this path.
/// - On a GSP client the kernel half does no hardware work at all and RPCs it straight to
///   us: `kchannelCtrlCmdGpFifoSchedule_IMPL` → `if (IS_VIRTUAL(pGpu) || IS_GSP_CLIENT(pGpu))
///   NV_RM_RPC_CONTROL(...)` (`ogkm-580:
///   src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:3085-3131`).
/// - The host direction is still the `a06c` form, because the channel *the isolate*
///   allocates does live in a TSG ([`NVA06C_CTRL_CMD_GPFIFO_SCHEDULE`], and
///   `kayfabe_isolate_host`'s `schedule` issues it on the group).
///
/// ⇒ guest asks `0xa06f0103` on a TSG-less channel; we answer; the host act, when there is
/// one, is `0xa06c0101` on a group. Same requirement, three different objects.
pub const NVA06F_CTRL_CMD_GPFIFO_SCHEDULE: u32 = 0xa06f_0103;

/// ★★ The status the guest's own driver documents for [`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`]
/// — and `NV_ERR_NOT_SUPPORTED` (`0x56`) is **not among them**.
///
/// `ogkm-580: ctrla06fgpfifo.h:59-64` lists exactly `NV_OK`,
/// `NV_ERR_INVALID_OBJECT_HANDLE`, `NV_ERR_INVALID_STATE`, `NV_ERR_INVALID_OPERATION`.
///
/// ★★★ That makes `0x56` here a **signature**, not an answer: it is what
/// `kayfabe_gsp::GspFsm::answer` posts when *nobody claimed the command*, and it is the
/// value the bench guest printed for six weeks —
/// `_memmgrMemUtilsScrubInitScheduleChannel: Unable to schedule channel, status: 56`.
/// A refusal that this port *decided* must therefore not reuse it, or the guest's own log
/// cannot distinguish "I examined your channel and refused" from "no code path exists".
/// Refusals from this port's schedule policy use [`GPFIFO_SCHEDULE_REFUSED_STATUS`].
/// ⚠ The three non-zero values are `ogkm-580: nvstatuscodes.h:80,85,93` and **not** what a
/// plausible-looking guess produces: `NV_ERR_INVALID_STATE` is `0x40`, not `0x39`, and
/// `NV_ERR_INVALID_OBJECT_HANDLE` is `0x33`, not `0x1e`. Both wrong values were written
/// here first and caught only by reading the table.
pub const GPFIFO_SCHEDULE_DOCUMENTED_STATUSES: &[u32] = &[
    0x0,  // NV_OK
    0x33, // NV_ERR_INVALID_OBJECT_HANDLE
    0x40, // NV_ERR_INVALID_STATE
    0x38, // NV_ERR_INVALID_OPERATION
];

/// `NV_ERR_INVALID_STATE` — the status this port answers when it has **looked at** the
/// channel and declines to schedule it.
///
/// Chosen over `NV_ERR_NOT_SUPPORTED` for the reason in
/// [`GPFIFO_SCHEDULE_DOCUMENTED_STATUSES`]: it is in the control's documented set, so the
/// guest's own error path treats it as an answer rather than as an absent one, and a reader
/// of the guest dmesg can tell the two apart by the hex alone.
pub const GPFIFO_SCHEDULE_REFUSED_STATUS: u32 = 0x40;

/// `NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:69-73`.
///
/// Three `NvBool`s, i.e. **three bytes**. `NvBool` is one byte, so this struct is 3 long
/// and 1 aligned — passing four bytes is a different `paramsSize` and a different request.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GpfifoScheduleParams {
    /// `NvBool bEnable` @ +0 — schedule the channel and add it to its runlist.
    pub b_enable: u8,
    /// `NvBool bSkipSubmit` @ +1.
    pub b_skip_submit: u8,
    /// `NvBool bSkipEnable` @ +2.
    pub b_skip_enable: u8,
}

impl GpfifoScheduleParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS";
    /// `sizeof`.
    pub const SIZE: usize = 3;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(bytes, Self::C_NAME, Self::SIZE, 0, &[self.b_enable])?;
        put(bytes, Self::C_NAME, Self::SIZE, 1, &[self.b_skip_submit])?;
        put(bytes, Self::C_NAME, Self::SIZE, 2, &[self.b_skip_enable])
    }

    /// Whether this is the request the port's FIFO model can act on at all — `bEnable`
    /// either way, and **neither** skip flag set.
    #[must_use]
    pub fn is_modelled(&self) -> bool {
        self.b_skip_submit == 0 && self.b_skip_enable == 0
    }
}

/// Why a [`GpfifoScheduleParams`] image was refused.
///
/// ★ The variants are the port's whole refusal vocabulary for `0xa06f0103`, and they are
/// **named** rather than collapsed to one status precisely so a boot's report can say
/// which one fired. `NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` has no `[OUT]` field, so nothing
/// downstream can recover a wrong decode by looking at the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpfifoScheduleError {
    /// Fewer than [`GpfifoScheduleParams::SIZE`] bytes of params.
    ShortParams {
        /// What arrived.
        got: usize,
    },
    /// A byte that is not a valid `NvBool`. ⊘ Deliberately a **decode** failure and not a
    /// policy question, for [`crate::l2evict`]'s reason: `NvBool` is `NV_TRUE`/`NV_FALSE`
    /// and a third value means the image is not the struct we think it is, which no
    /// amount of policy can repair.
    NonBoolean {
        /// The C field name.
        field: &'static str,
        /// The byte that is neither 0 nor 1.
        value: u8,
    },
    /// `bSkipSubmit` and/or `bSkipEnable` set — the **enabled-versus-scheduled split**.
    ///
    /// ★★★ This is the part of the control this port does not model, named. RM documents
    /// the two flags as separating "in the runlist" from "will actually be run"
    /// (`ogkm-580: ctrla06fgpfifo.h:44-55`); this port's [`ExecPlane`-side] state is a
    /// single membership and has no third value to move to. ⊘ Serving these by ignoring
    /// them would be the silent-`NV_OK` failure with extra steps: the guest would have
    /// asked for a channel that is scheduled and *not* submitted, and got one that is
    /// both.
    ///
    /// [`ExecPlane`-side]: this is `kayfabe_core::gpu::ExecPlane`; named in prose because
    /// `kayfabe-abi` does not depend on the core.
    UnmodelledSkip {
        /// `bSkipSubmit` as it arrived.
        b_skip_submit: u8,
        /// `bSkipEnable` as it arrived.
        b_skip_enable: u8,
    },
}

impl core::fmt::Display for GpfifoScheduleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GpfifoScheduleError::ShortParams { got } => write!(
                f,
                "{} needs {} bytes of params, got {got}",
                GpfifoScheduleParams::C_NAME,
                GpfifoScheduleParams::SIZE
            ),
            GpfifoScheduleError::NonBoolean { field, value } => {
                write!(f, "{field} = {value:#04x} is not an NvBool")
            }
            GpfifoScheduleError::UnmodelledSkip {
                b_skip_submit,
                b_skip_enable,
            } => write!(
                f,
                "bSkipSubmit={b_skip_submit} bSkipEnable={b_skip_enable}: this port does not \
                 model a channel that is scheduled but not submitted (or vice versa)"
            ),
        }
    }
}

impl core::error::Error for GpfifoScheduleError {}

/// Decode an `NVA06F_CTRL_GPFIFO_SCHEDULE_PARAMS` image.
///
/// # Errors
/// [`GpfifoScheduleError`], by variant.
pub fn decode_gpfifo_schedule(params: &[u8]) -> Result<GpfifoScheduleParams, GpfifoScheduleError> {
    if params.len() < GpfifoScheduleParams::SIZE {
        return Err(GpfifoScheduleError::ShortParams { got: params.len() });
    }
    for (field, value) in [
        ("bEnable", params[0]),
        ("bSkipSubmit", params[1]),
        ("bSkipEnable", params[2]),
    ] {
        if value > 1 {
            return Err(GpfifoScheduleError::NonBoolean { field, value });
        }
    }
    let out = GpfifoScheduleParams {
        b_enable: params[0],
        b_skip_submit: params[1],
        b_skip_enable: params[2],
    };
    if !out.is_modelled() {
        return Err(GpfifoScheduleError::UnmodelledSkip {
            b_skip_submit: out.b_skip_submit,
            b_skip_enable: out.b_skip_enable,
        });
    }
    Ok(out)
}

/// The reply body a real GA106's own GSP sends for `0xa06f0103`: **the request's three
/// params bytes, unchanged**.
///
/// ★★★ `[measured]` on a real GA106 —
/// `traces/real_ga106/rpc_transcript_real_ga106.txt:59`, `cmd=0xa06f0103 psize=3
/// gspst=0x0 head=01 00 00`. The guest sent `bEnable=1` and got `01 00 00` back with
/// status `NV_OK`.
///
/// ⊘ **Not** taken from the C artifact's captured table, whose row for this id is
/// `{0xa06f0103u, 0x0u, 3u, 0u, ctl_a06f0103}` — `dlen = 0`, an **empty body**
/// (`C: src/qemu/mode2_initctrl_ga106.h:6234 = 0xa06f0103`). That row is one of the 11/56 the FIFTH
/// LIMIT contradicts (`crates/kayfabe-abi/src/oracle.rs:39`), and an empty capture is
/// evidence of nothing. The C's *status* is corroborated by hardware; its *body* is not,
/// and only the hardware trace is cited for the bytes.
///
/// ★ Why echoing matters even though every field is `[IN]`: the GSP transport copies the
/// reply's params back over the caller's own struct whenever `paramsSize != 0`
/// (`ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`), so a zero-filled body would
/// silently clear the caller's `bEnable`. `NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION` taught
/// this port that lesson; hardware happens to agree here.
#[must_use]
pub fn encode_gpfifo_schedule(req: &GpfifoScheduleParams) -> Vec<u8> {
    let mut out = vec![0u8; GpfifoScheduleParams::SIZE];
    req.encode_into(&mut out)
        .expect("SIZE bytes is exactly what encode_into needs");
    out
}

// =====================================================================================
// ★★★ E9 — `NVA06F_CTRL_CMD_BIND`, the control that gives a channel a RUNLIST
// =====================================================================================

/// `NVA06F_CTRL_CMD_BIND` = `0xa06f0104`, issued **on the channel object** —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:96`.
///
/// ## ★★★ Who sends this, and why it is not the function whose name matches
///
/// ⊘ `kchannelCtrlCmdBind_IMPL` is the **receiving** half — what RM runs when it is the one
/// being asked. On a GSP client the guest kernel never reaches it. The sender is
/// [`kchannelBindToRunlist_IMPL`] (`ogkm-580:
/// src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2762-2785`), which on
/// `IS_GSP_CLIENT(pGpu)` builds `NVA06F_CTRL_BIND_PARAMS` **itself** and
/// `NV_RM_RPC_CONTROL`s it to us:
///
/// ```text
/// params.engineType = gpuGetNv2080EngineType(localRmEngineType);
/// NV_RM_RPC_CONTROL(pGpu, …, NVA06F_CTRL_CMD_BIND, &params, sizeof(params), status);
/// NV_ASSERT_OK_OR_RETURN(status);
/// ```
///
/// ★ Two consequences the shape of this port depends on:
///
/// 1. **Our answer is load-bearing, immediately.** `NV_ASSERT_OK_OR_RETURN` means a
///    non-`NV_OK` stops the guest before `kfifoRunlistSetIdByEngine_HAL`, so a refusal is
///    seen. An undeserved `NV_OK` is worse than a refusal: the guest proceeds to assign a
///    runlist id for an engine we never agreed to, which is the C-era
///    `dma_copy_class_alloc_params` defect (`engineType=0` → wrong runlist) with the
///    error moved one layer out.
/// 2. **Not every bind reaches us.** `if ((engineDesc == ENG_SW) || (engineDesc == ENG_BUS))
///    return NV_OK;` short-circuits before the RPC (`:2762-2765`), so a port that counts
///    binds will count fewer than the guest performed. ⊘ That is not a lost message.
///
/// ## `[measured]` What a real GA106 was asked
///
/// `traces/real_ga106/rpc_transcript_real_ga106.txt:63` —
/// `cmd=0xa06f0104 psize=4 gspst=0x0 head=0b 00 00 00`, i.e. **`engineType = 11`**.
/// With `NV2080_ENGINE_TYPE_COPY0 = 9` (`ogkm-580:
/// src/common/sdk/nvidia/inc/class/cl2080_notification.h:293`) and the `COPY(i)` macro at
/// `:398`, that is **`COPY2`** — `RmInitAdapter` binds the CeUtils scrubber channel to the
/// third copy engine, not the first.
///
/// ⚠ And the C's captured row for this id is one of the **11 empty ones**
/// (`crates/kayfabe-abi/src/oracle.rs`, `psize 4, dlen 0`): the C would have decoded it as
/// `engineType = 0`, which is `NV2080_ENGINE_TYPE_NULL`. Nine of those eleven rows are
/// contradicted by hardware and this is one of them.
pub const NVA06F_CTRL_CMD_BIND: u32 = 0xa06f_0104;

/// The statuses `NVA06F_CTRL_CMD_BIND` **documents** —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:91-95`.
///
/// ★★ **`NV_ERR_NOT_SUPPORTED` (`0x56`) is not among them**, exactly as for
/// [`NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`] — and for the same reason it must not be used as a
/// refusal here: `0x56` is `kayfabe_gsp::GspFsm::answer`'s signature for *"nobody claimed
/// this command"*, and the guest prints the raw hex. Reusing it erases the only difference
/// a reader has between a decision and an absence.
///
/// ⊘ **And this list is NOT the set of answers to choose from — see
/// [`BIND_STATUSES_THE_CODE_PRODUCES`].** The header is incomplete relative to the driver's
/// own code, and building a gate out of it would have forced a wrong answer.
///
/// ⚠ Values read from `ogkm-580: kernel-open/common/inc/nvstatuscodes.h:60,93`, not
/// guessed: `NV_ERR_INVALID_ARGUMENT` is `0x1F` and `NV_ERR_INVALID_STATE` is `0x40`.
pub const BIND_DOCUMENTED_STATUSES: &[u32] = &[
    0x0,  // NV_OK
    0x1f, // NV_ERR_INVALID_ARGUMENT
    0x40, // NV_ERR_INVALID_STATE
];

/// ★★★ The statuses the **code** actually produces on the bind path — a strict superset of
/// [`BIND_DOCUMENTED_STATUSES`].
///
/// ## ⚠ Why the doc list could not be the gate
///
/// A first cut of this module made [`BIND_DOCUMENTED_STATUSES`] the acceptance set for
/// refusals, on the same reasoning that made it right for
/// [`GPFIFO_SCHEDULE_DOCUMENTED_STATUSES`]. Walking the call path
/// (`ogkm-580: kernel_channel.c:3069-3133` → `gpu.c:5274-5295` →
/// `kernel_fifo_gm107.c:447-488` → `:672-759`) shows the header is **wrong by omission**:
/// the status for *"a structurally valid engine this chip does not have"* is
/// `NV_ERR_OBJECT_NOT_FOUND` (`0x57`), returned from
/// `ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:736`, and it
/// is **not in the header's list**.
///
/// ⇒ A gate built from the header would have rejected the true answer and admitted only
/// false ones — `mock_fidelity_both_directions`' too-strict half, arriving through a
/// citation that was perfectly real. ★ The header is a *claim about* the code, not the
/// code.
pub const BIND_STATUSES_THE_CODE_PRODUCES: &[u32] = &[
    0x0,  // NV_OK
    0x1f, // NV_ERR_INVALID_ARGUMENT   — gpu.c:5294, and gm107.c:465 (runqueue 1)
    0x40, // NV_ERR_INVALID_STATE      — gm107.c:410, :425-427 (runlist already fixed)
    0x57, // NV_ERR_OBJECT_NOT_FOUND   — gm107.c:736 (engine absent from THIS chip's table)
];

/// `NV_ERR_OBJECT_NOT_FOUND` — what this port answers when the request names a
/// **structurally valid engine that this device never advertised**.
///
/// `[inferred]`, and the inference is named rather than buried: the GSP-RM firmware is not
/// in the open tree, so what a real GSP answers cannot be read. What *can* be read is that
/// `kchannelBindToRunlist_IMPL` RPCs **only** when `IS_GSP_CLIENT(pGpu)` and otherwise falls
/// through to `kfifoRunlistSetIdByEngine_HAL`
/// (`ogkm-580: kernel_channel.c:2767,2789`) — a structure that only makes sense if the same
/// function is the RPC's receiver on the GSP side. Down that path,
/// `kfifoEngineInfoXlate_GM107` linear-scans **this GPU's own engine-info list** and returns
/// `NV_ERR_OBJECT_NOT_FOUND` when nothing matches
/// (`ogkm-580: kernel_fifo_gm107.c:734-737`). That list is built from the very device-info
/// table this port serves, which is why refusing an engine we never advertised is the
/// *faithful* answer rather than an invented one.
///
/// ⊘ **Not `NV_ERR_INVALID_ARGUMENT`.** That is what CPU-RM produces **locally, before any
/// RPC is sent**, for an ordinal that is not a bindable engine type at all
/// (`gpuXlateClientEngineIdToEngDesc`, `ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:5294`) —
/// against a **static, chip-independent** table that `pGpu` is passed to and never read
/// from. Such a bind never reaches us, so answering with its status would be describing a
/// check we do not perform.
///
/// ⚠ ★ **`0x57` is one away from `0x56`**, the FSM's *"nobody claimed this"* signature, and
/// the guest prints raw hex. They mean opposite things — *"I looked and this device has no
/// such engine"* versus *"no code path exists"* — so a reader of a boot log must not
/// transpose them. Recorded because this port has already lost weeks to `status: 56`.
pub const BIND_UNKNOWN_ENGINE_STATUS: u32 = 0x57;

/// `NV_ERR_INVALID_STATE` — what this port answers when the request is well-formed and
/// names a real engine, but the **channel** could not be routed.
pub const BIND_REFUSED_STATUS: u32 = 0x40;

/// `NVA06F_CTRL_BIND_PARAMS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrla06f/ctrla06fgpfifo.h:98-101`.
///
/// One `NvU32 engineType`, and the header says it is *"an `NV2080_ENGINE_TYPE` value"*.
///
/// ⚠ ★ **The numbering space is part of the type and is easy to get wrong.** The engine
/// tables this port serves through the device-info path carry `RM_ENGINE_TYPE`
/// (`FifoDeviceEntry::engine_data[engine_info_type::RM_ENGINE_TYPE]`), which is a
/// *different enum* that RM converts with `gpuGetNv2080EngineType` /`gpuGetRmEngineType`.
/// Comparing one against the other without converting is the same species of defect as
/// reading a VA as a GPA, and it would be silent.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BindParams {
    /// `NvU32 engineType` @ +0 — an `NV2080_ENGINE_TYPE_*` ordinal.
    pub engine_type: u32,
}

impl BindParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NVA06F_CTRL_BIND_PARAMS";
    /// `sizeof` — one `NvU32`.
    pub const SIZE: usize = 4;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.engine_type.to_le_bytes(),
        )
    }
}

/// ★★★ Convert an `NV2080_ENGINE_TYPE` ordinal to the `RM_ENGINE_TYPE` the engine tables
/// are written in — `None` if it names nothing bindable.
///
/// ## ⊘ Why this function has to exist, with the counter-example
///
/// The two spaces are **not** the same enum, and the driver says so in as many words
/// (`ogkm-580: src/nvidia/src/kernel/gpu/gpu_engine_type.c:37-43`): *"Rm internally uses RM
/// engine type instead of NV2080 engine types … When ENGINE_TYPE cross RM boundary, through
/// control calls or RPC calls, we will need to convert the engine types."*
///
/// They agree on exactly the two ranges this port cares about most —
/// `GR0..GR7 = 0x01..0x08` and `COPY0..COPY9 = 0x09..0x12` — which is precisely what makes
/// a raw integer comparison look correct in every test anyone would think to write.
/// **Above that they collide:**
///
/// | raw | as `NV2080_ENGINE_TYPE` | as `RM_ENGINE_TYPE` |
/// |---|---|---|
/// | `0x13` | `NVDEC0` (`cl2080_notification.h:303-304`) | `COPY10` (`gpu_engine_type.h:53`) |
/// | `0x22` | `SW` (`cl2080_notification.h:320`) | `COPY25`-region / not `SW` |
/// | `0x2d` | — | `SW` (`gpu_engine_type.h:79`) |
/// | `0x34` | `COPY10` (`cl2080_notification.h:342`) | `COPY43`-region / not a CE |
///
/// ⇒ A bind for `NVDEC0` compared raw against an RM-space table would be **accepted as a
/// bind to the eleventh copy engine.** Same species as reading a VA as a GPA, and just as
/// silent.
///
/// ★ It also confirms, rather than assumes, which space our own tables are in:
/// `ga10x.rs`'s `SOFTWARE` row carries `0x2d`, which is `RM_ENGINE_TYPE_SW` and is **not**
/// `NV2080_ENGINE_TYPE_SW` (`0x22`). The shipped table is in RM space, which is what a
/// GSP client expects — the fetched table is `portMemCopy`'d verbatim with no translation
/// (`ogkm-580: src/nvidia/src/kernel/gpu/fifo/kernel_fifo.c:2104-2108`). ⚠ The **vGPU**
/// path is the exception and converts in place (`:1934-1950`, guarded by `IS_VIRTUAL`), so
/// a future vGPU posture must revisit this and not inherit it.
///
/// ## What is modelled, and what is refused
///
/// Only the identity ranges plus `SW`. `None` for everything else — including the video
/// engines and the second CE decade, which this port advertises no rows for. ⊘ Deliberately
/// **not** a transcription of all ~57 cases of
/// `gpuGetRmEngineType_IMPL` (`gpu_engine_type.c:50-149`): a conversion this port cannot
/// then act on would be a too-capable double, and every unmodelled ordinal reaching
/// [`BIND_UNKNOWN_ENGINE_STATUS`] is the honest answer for a device with no such engine.
#[must_use]
pub fn nv2080_to_rm_engine_type(nv2080: u32) -> Option<u32> {
    match nv2080 {
        // GR0..GR7 — identical in both spaces (`gpu_engine_type.c:62-69` vs `:172-179`).
        0x01..=0x08 => Some(nv2080),
        // COPY0..COPY9 — identical in both spaces (`:70-79` vs `:180-189`).
        ENGINE_TYPE_COPY0..=0x12 => Some(nv2080),
        // SW — the one row where the two spaces DISAGREE and this port has a table entry.
        NV2080_ENGINE_TYPE_SW => Some(RM_ENGINE_TYPE_SW),
        _ => None,
    }
}

/// `NV2080_ENGINE_TYPE_SW` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/cl2080_notification.h:320`.
pub const NV2080_ENGINE_TYPE_SW: u32 = 0x0000_0022;

/// `RM_ENGINE_TYPE_SW` — `ogkm-580: src/nvidia/inc/kernel/gpu/gpu_engine_type.h:79`.
///
/// ⚠ `0x2d`, not `0x22`. The pair is the cheapest available proof that the two engine-type
/// spaces are different enums; see [`nv2080_to_rm_engine_type`].
pub const RM_ENGINE_TYPE_SW: u32 = 0x0000_002d;

/// `RM_ENGINE_TYPE_COPY0` — `ogkm-580: gpu_engine_type.h:43`.
///
/// ⚠ Numerically equal to [`ENGINE_TYPE_COPY0`] and **not the same constant**. See
/// [`nv2080_to_rm_engine_type`] for the collision table; the two agree here and diverge
/// nine ordinals later, which is exactly the shape that makes one name for both wrong.
pub const RM_ENGINE_TYPE_COPY0: u32 = 0x0000_0009;

/// `RM_ENGINE_TYPE_COPY_SIZE` — how many copy engines the RM enum reserves
/// (`ogkm-580: gpu_engine_type.h:131`).
pub const RM_ENGINE_TYPE_COPY_SIZE: u32 = 20;

/// ★★★ *"Which copy engine is this, in **RM** engine space?"* — `RM_ENGINE_TYPE_IS_COPY`
/// then `RM_ENGINE_TYPE_COPY_IDX`, transcribed as one function
/// (`ogkm-580: gpu_engine_type.h:139-141`).
///
/// # ⊘ This is NOT [`copy_index_of_engine_type`], and the difference is a real bug
///
/// In `NV2080_ENGINE_TYPE` space the copy engines are two discontiguous decades with
/// `NVDEC0` sitting in the gap; in **RM** space `RM_ENGINE_TYPE_COPY0..COPY19` are one
/// unbroken run `0x09..=0x1c` (`gpu_engine_type.h:43-62`). Feeding an RM-space value to the
/// `NV2080` inverse answers `None` for `COPY10..COPY19` — a real engine reported as *"not a
/// copy engine"* — and feeding an `NV2080` value to this one turns `NVDEC0` (`0x13`) into
/// **copy engine 10**. Both directions are silent, which is why the two live side by side
/// with this paragraph between them.
///
/// ⊘ `None` means *"not a copy engine"* and never *"copy engine 0"*, for
/// [`copy_index_of_engine_type`]'s reason: CE0 is one of the two indices whose non-stall
/// vector this port's captured table publishes as `INVALID`, so a zero default would fake
/// precisely the answer that must never be faked.
#[must_use]
pub const fn rm_copy_index_of_engine_type(rm_engine_type: u32) -> Option<u32> {
    if rm_engine_type >= RM_ENGINE_TYPE_COPY0
        && rm_engine_type < RM_ENGINE_TYPE_COPY0 + RM_ENGINE_TYPE_COPY_SIZE
    {
        Some(rm_engine_type - RM_ENGINE_TYPE_COPY0)
    } else {
        None
    }
}

/// Decode a [`BindParams`] image.
///
/// ⊘ There is **no validity check here and that is deliberate**: every 32-bit value is a
/// syntactically valid `NvU32`, so unlike [`decode_gpfifo_schedule`] — whose `NvBool`s have
/// only two legal bytes — this decode cannot fail on anything but length. Whether the
/// ordinal names an engine is a *policy* question with a different answer per device, and
/// answering it here would bake one device's engine set into the ABI crate.
///
/// # Errors
/// [`GpfifoScheduleError::ShortParams`] if fewer than [`BindParams::SIZE`] bytes.
pub fn decode_bind(params: &[u8]) -> Result<BindParams, GpfifoScheduleError> {
    if params.len() < BindParams::SIZE {
        return Err(GpfifoScheduleError::ShortParams { got: params.len() });
    }
    Ok(BindParams {
        engine_type: u32::from_le_bytes([params[0], params[1], params[2], params[3]]),
    })
}

/// Encode a [`BindParams`] for the reply body.
///
/// ★ The reply carries the request's own params back, for
/// [`encode_gpfifo_schedule`]'s reason: the GSP transport copies the reply's params over
/// the caller's struct whenever `paramsSize != 0` (`ogkm-580:
/// src/nvidia/src/kernel/vgpu/rpc.c:11085-11090`), so a zero-filled body would rewrite the
/// caller's `engineType` to `NV2080_ENGINE_TYPE_NULL` behind its back. `[measured]`
/// hardware agrees: the real GA106 reply body IS the request's four bytes.
#[must_use]
pub fn encode_bind(req: &BindParams) -> Vec<u8> {
    let mut out = vec![0u8; BindParams::SIZE];
    req.encode_into(&mut out)
        .expect("SIZE bytes is exactly what encode_into needs");
    out
}

/// `NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrlc36f.h:79`.
///
/// ★★ **This is the rung's oracle.** The token is a hardware-assigned identity for the
/// channel — RM does not invent it and we cannot compute it — so a value coming back out
/// of this control is a fact about the GPU's channel RAM, not about our code. Issued **on
/// the channel object** (not the TSG). Its params are a single `NvU32 workSubmitToken`
/// (`ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrlc36f.h:83-85`), which is why there is
/// no struct here: four bytes, `[OUT]`.
///
/// ★ The value is structural, not opaque: `(runlistId << 16) | chid`, measured in the C
/// (`C: docs/design/mode2_doorbell_chid.md:337-345`). This port does **not** decompose
/// it — [`kayfabe_isolate::RmBackend::alloc_channel`] returns it whole and
/// `ring_doorbell` stores it whole — but the structure is why a token is *evidence*: a
/// channel that was never bound to a runlist cannot have one.
pub const NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN: u32 = 0xc36f_0108;

/// `sizeof(NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN_PARAMS)` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrlc36f.h:83-85`.
pub const WORK_SUBMIT_TOKEN_PARAMS_SIZE: usize = 4;

// =====================================================================================
// ★★★ The E3 instrument — RM's OWN channel-ID manager, asked directly
// =====================================================================================

/// `NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS` = `0x20801119`, on the **subdevice** —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:891`.
///
/// ★★★ **This is the only control in the tree that reports a channel's `(runlistId,
/// chid)` without going anywhere near a work-submit token**, which is exactly why it is
/// here. `subdeviceCtrlCmdFifoGetAllocatedChannels_IMPL` →
/// `kfifoGetAllocatedChannelMask_IMPL` (`kernel_fifo.c:3371-3443`) takes `runlistId` as an
/// **input**, walks `chId` over that runlist's `CHID_MGR`, and sets a bit for every
/// `kfifoChidMgrGetKernelChannel` that is non-NULL. Diffed across one channel allocation
/// it names the `(runlist, chid)` RM just assigned — from RM's allocator, not from the
/// token, not from our decoder, and not from our decoder's inverse. `E3` is the increment
/// whose *wrong answer is silent*, so its expected value had to come from somewhere the
/// answer could not have leaked into.
///
/// ⊘ `RMCTRL_FLAGS_PRIVILEGED` (`flags = 0x4u`, `g_subdevice_nvoc.c:5086`), i.e.
/// admin-only (`control.h:196-202`). A capability-less isolate is refused it, by design —
/// this is a **bench instrument**, not a production path, and nothing on the guest path
/// issues it.
pub const NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS: u32 = 0x2080_1119;

/// How many channels `NV2080_CTRL_FIFO_GET_ALLOCATED_CHANNELS_PARAMS::bitMask` covers —
/// `NV2080_CTRL_FIFO_GET_ALLOCATED_CHANNELS_MAX_CHANNELS`, `ctrl2080fifo.h:898`.
pub const ALLOCATED_CHANNELS_MAX: usize = 4096;

/// `sizeof(NV2080_CTRL_FIFO_GET_ALLOCATED_CHANNELS_PARAMS)` — `NvU32 runlistId` followed
/// by `NvU32 bitMask[MAX/32]` (`ctrl2080fifo.h:902-905`). No pointers, no alignment
/// padding: 4 + 512.
pub const ALLOCATED_CHANNELS_PARAMS_SIZE: usize = 4 + ALLOCATED_CHANNELS_MAX / 8;

/// `NV2080_CTRL_CMD_FIFO_GET_INFO` = `0x20801109`, on the **subdevice** —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080fifo.h:262`.
///
/// ★★ Unlike [`NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS`] this one is
/// `RMCTRL_FLAGS_NON_PRIVILEGED` (`flags = 0x30008u`, `g_subdevice_nvoc.c:4906`), so any
/// client may ask it — which matters, because the E3 census needs it and the census must
/// stay reproducible by someone who is not root.
pub const NV2080_CTRL_CMD_FIFO_GET_INFO: u32 = 0x2080_1109;

/// `sizeof(NV2080_CTRL_FIFO_GET_INFO_PARAMS)` — `NvU32 fifoInfoTblSize`, then 256
/// `{NvU32 index; NvU32 data;}` pairs, then `NvU32 engineType`
/// (`ctrl2080fifo.h:268-278`). All `NvU32`, so no padding: 4 + 2048 + 4.
pub const FIFO_GET_INFO_PARAMS_SIZE: usize = 4 + 256 * 8 + 4;

/// `NV2080_CTRL_FIFO_INFO_INDEX_IS_PER_RUNLIST_CHANNEL_RAM_SUPPORTED` —
/// `ctrl2080fifo.h:231`. **1** = each runlist has its own channel RAM and therefore its
/// own chid namespace; **0** = one global `CHID_MGR` for the whole device.
///
/// ★★★ Which it is decides whether `(GpuId, VChid)` is a channel identity at all, so it
/// is measured rather than assumed. `[measured]` = **0** on RTX 3060 / GA106 /
/// 580.159.04 — see `docs/design/doorbell_token_encoding.md` §4.
pub const FIFO_INFO_INDEX_IS_PER_RUNLIST_CHANNEL_RAM_SUPPORTED: u32 = 7;

/// `NV2080_CTRL_FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE` —
/// `ctrl2080fifo.h:233`. Reads `params.engineType`, translates it to a **runlist id**
/// through `kfifoEngineInfoXlate_HAL(… ENGINE_INFO_TYPE_RUNLIST …)` and returns that
/// runlist's in-use channel-group count (`kernel_fifo_ctrl.c:299-306`).
///
/// ★★★ **The E3 instrument for the token's UPPER field.** Allocating one channel on
/// engine *X* raises this count for exactly the engines that share *X*'s runlist, so
/// diffing it across the sweep recovers the engines→runlist **partition** with no
/// reference to a work-submit token. The runlist *ids* are not exposed to an
/// unprivileged client (`GET_DEVICE_INFO_TABLE` is `KERNEL_PRIVILEGED`), so the partition
/// is what is measurable and the census reports exactly that.
pub const FIFO_INFO_INDEX_CHANNEL_GROUPS_IN_USE_PER_ENGINE: u32 = 9;

// =====================================================================================
// Device-local memory — the only kind a ring, a USERD and a semaphore can be built from
// =====================================================================================

/// `NV01_MEMORY_LOCAL_USER` — `ogkm-580: src/common/sdk/nvidia/inc/nvos.h` class `0x0040`,
/// allocated with [`NvMemoryAllocationParams`].
///
/// ★ Why not the sysmem path [`crate::bringup`] already has: `alloc_sysmem` issues
/// `NV_ESC_RM_ALLOC_MEMORY` with `NVOS02_FLAGS_MAPPING_NO_MAP`, which deliberately makes
/// the object **un-CPU-mappable** — right for a data buffer the GPU alone touches, and
/// exactly wrong for a ring whose whole purpose is that both sides write it. This class
/// takes the `NV_ESC_RM_ALLOC` path and produces an object [`NV_ESC_RM_MAP_MEMORY`] can
/// map. It is what the C's proven host channel allocates for its pushbuffer, GPFIFO,
/// semaphore and USERD (`C: src/qemu/nvkvm_gpu_emul.c:9488-9495`, `:7278-7298`).
pub const NV01_MEMORY_LOCAL_USER: u32 = 0x0040;

/// `NV_MEMORY_ALLOCATION_PARAMS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:1608-1642`. 128 bytes.
///
/// Only the four fields a device-local allocation needs are named; the rest are `[IN]`
/// zero or `[OUT]`. `pitch`, `offset`, `limit` and `address` are `[IN/OUT]` or `[OUT]`
/// and are deliberately not surfaced — reading back a returned `address` would be reading
/// back a host CPU address, which is the representation `kayfabe_linux_raw`'s §4.2.1
/// refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NvMemoryAllocationParams {
    /// `NvU32 owner` @ +0 — an owner tag. The C passes the client handle
    /// (`C: src/qemu/nvkvm_gpu_emul.c:7283`).
    pub owner: u32,
    /// `NvU32 type` @ +4 — `NVOS32_TYPE_IMAGE` = 0
    /// (`ogkm-580: src/common/sdk/nvidia/inc/nvos.h:884`).
    pub kind: u32,
    /// `NvU32 attr` @ +24 — see [`ATTR_CONTIGUOUS_VIDMEM`].
    pub attr: u32,
    /// `NvU64 size` @ +64 — `[IN/OUT]`; RM may round it up.
    pub size: u64,
    /// `NvU64 alignment` @ +72.
    pub alignment: u64,
}

impl NvMemoryAllocationParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NV_MEMORY_ALLOCATION_PARAMS";
    /// `sizeof`.
    pub const SIZE: usize = 128;
    /// `alignof`.
    pub const ALIGN: usize = 8;

    /// Encode into a **zeroed** little-endian image of at least [`Self::SIZE`] bytes.
    /// The length is checked up front, for the reason
    /// [`ChannelAllocParams::encode_into`] states.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        let n = Self::C_NAME;
        let s = Self::SIZE;
        if bytes.len() < s {
            return Err(AbiError::Truncated {
                c_name: n,
                need: s,
                got: bytes.len(),
            });
        }
        put(bytes, n, s, 0, &self.owner.to_le_bytes())?;
        put(bytes, n, s, 4, &self.kind.to_le_bytes())?;
        put(bytes, n, s, 24, &self.attr.to_le_bytes())?;
        put(bytes, n, s, 64, &self.size.to_le_bytes())?;
        put(bytes, n, s, 72, &self.alignment.to_le_bytes())
    }
}

/// `NVOS32_ATTR_PHYSICALITY_CONTIGUOUS` in field `28:27` (value 2,
/// `ogkm-580: src/common/sdk/nvidia/inc/nvos.h:1078-1081`) OR
/// `NVOS32_ATTR_LOCATION_VIDMEM` in field `26:25` (value 0, `:1067-1068`).
///
/// ★ Like every `NVOS*` flag, these are **field ranges with values in them**, not bit
/// masks — the mistake `crate::bringup::NVOS02_FLAGS_LOCATION_PCI` records having made
/// against real hardware. `_VIDMEM` being zero is what "device-local" encodes as.
///
/// Contiguous is a *requirement* here, not a preference: a GPFIFO ring, a USERD block and
/// a pushbuffer are addressed as flat spans by hardware.
pub const ATTR_CONTIGUOUS_VIDMEM: u32 = 2 << 27;

// =====================================================================================
// USERD, the GPFIFO ring, and the doorbell
// =====================================================================================

/// Byte offset of `GP_GET` within USERD — `NV_RAMUSERD_GP_GET` is dword 34
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_ram.h:37`, the field range
/// `(34*32+31):(34*32+0)`), so `34 * 4`.
///
/// ★★ **Hardware writes this and nothing else does.** It is the consume cursor: the GPU's
/// host unit advances it as it fetches GPFIFO entries. That makes it the one value in this
/// whole module that can serve as evidence — we write `GP_PUT`, and if `GP_GET` moves to
/// meet it, something outside this process read the ring. The C artifact reads exactly
/// these two offsets for exactly that reason (`C: src/qemu/nvkvm_gpu_emul.c:4199-4202`,
/// *"USERD: GP_GET@0x88, GP_PUT@0x8C"*).
pub const USERD_GP_GET: u64 = 34 * 4;

/// Byte offset of `GP_PUT` within USERD — `NV_RAMUSERD_GP_PUT`, dword 35
/// (`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_ram.h:38`).
///
/// The produce cursor: **we** write it, and writing it is what makes new ring entries
/// visible to hardware. It must be written *after* the entries it announces, which is the
/// release-fence seam `kayfabe_linux_raw::VolatileRegion` names in its own docs.
pub const USERD_GP_PUT: u64 = 35 * 4;

/// `NVC56F_GP_ENTRY__SIZE` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:267`.
pub const GP_ENTRY_SIZE: u64 = 8;

/// Build one GPFIFO entry pointing at `gpu_va` for `len_bytes` of pushbuffer.
///
/// The encoding, all from `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:266-284`:
///
/// - `GP_ENTRY0_GET` is `31:2` — bits `31:2` of the **address**, i.e. the low 32 bits of
///   a 4-byte-aligned address used as-is. Bit 0 is `FETCH` and bit 1 is unused, so an
///   address whose low two bits are not zero would set `FETCH_CONDITIONAL` by accident.
/// - `GP_ENTRY1_GET_HI` is `7:0` — bits `39:32` of the address. **Eight bits, not more**:
///   a GPFIFO entry cannot name a pushbuffer above 2^40, and silently truncating one is
///   the kind of failure that presents as the engine executing whatever else lives there.
/// - `GP_ENTRY1_LENGTH` is `30:10` — the pushbuffer length **in dwords**, so 21 bits =
///   at most 2^21 - 1 dwords.
/// - `GP_ENTRY1_SYNC` is `31:31`, left `PROCEED` (0).
///
/// Returns `None` — never a truncated entry — when any of those bounds is exceeded or
/// `len_bytes` is not a whole number of dwords. A GPFIFO entry that names the wrong
/// address or the wrong length does not fail: it runs.
#[must_use]
pub const fn gp_entry(gpu_va: u64, len_bytes: u64) -> Option<u64> {
    if !gpu_va.is_multiple_of(4) {
        return None;
    }
    if gpu_va >= 1 << 40 {
        return None;
    }
    if len_bytes == 0 || !len_bytes.is_multiple_of(4) {
        return None;
    }
    let dwords = len_bytes / 4;
    if dwords >= 1 << 21 {
        return None;
    }
    let entry0 = gpu_va & 0xFFFF_FFFC;
    let entry1 = ((gpu_va >> 32) & 0xFF) | (dwords << 10);
    Some(entry0 | (entry1 << 32))
}

/// The size of one channel's USERD — `NV_RAMUSERD_CHAN_SIZE`, i.e.
/// `1 << NV_RAMUSERD_BASE_SHIFT` (`ogkm-580:
/// src/common/inc/swref/published/maxwell/gm107/dev_ram.h:49-50`).
///
/// ★★ **Derived from the driver's own HAL choice, not from the chip's own header.**
/// `kfifoGetUserdSizeAlign` is halified in two ways in `g_kernel_fifo_nvoc.c`, and every
/// chip except `T234D`/`T264D` — GA106 included — falls to the `else` arm, which is
/// `kfifoGetUserdSizeAlign_GM107` (`ogkm-580:
/// src/nvidia/src/kernel/gpu/fifo/arch/maxwell/kernel_fifo_gm107.c:1553`, `*pSize =
/// 1<<NV_RAMUSERD_BASE_SHIFT`). So the number an Ampere channel's USERD is sized with is
/// Maxwell's, and reaching for `published/ampere/ga102/dev_ram.h` finds **no**
/// `NV_RAMUSERD` at all. `tests/oracle/pushbuffer_abi_oracle.c` compiles that function
/// rather than trusting this sentence.
///
/// ★ Both cursors must fit: [`USERD_GP_PUT`] is at `0x8C`, so a size of `512` leaves the
/// window closed over both. A model answering less would size a mapping that stops short
/// of the produce cursor.
pub const USERD_SIZE: u64 = 512;

/// One GPFIFO entry, decoded — the inverse of [`gp_entry`], and the read side of the
/// ring the guest fills.
///
/// ⊘ **A control entry decodes to `None`, not to a zero-length range.** `LENGTH == 0`
/// means entry1's low byte is `GP_ENTRY1_OPCODE` (`NOP`/`ILLEGAL`/`GP_CRC`/`PB_CRC`,
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:280-284`) and **not**
/// `GP_ENTRY1_GET_HI` — the two fields are the same eight bits. Reading the address out
/// of a control entry produces a plausible pointer into guest memory that the guest never
/// named, which is exactly the class of answer this port refuses to invent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpfifoEntry {
    /// The GPU virtual address of the method words (`GP_ENTRY0_GET` + `GP_ENTRY1_GET_HI`).
    pub gpu_va: u64,
    /// Their length in **bytes** (`GP_ENTRY1_LENGTH` is in dwords).
    pub len_bytes: u64,
    /// `GP_ENTRY1_LEVEL == _SUBROUTINE` — a nested pushbuffer segment rather than a
    /// top-level one. Carried, not acted on: it still names method words.
    pub subroutine: bool,
    /// `GP_ENTRY1_SYNC == _WAIT` — the host must drain before fetching this entry.
    pub sync_wait: bool,
}

/// Decode one 8-byte GPFIFO entry (entry0 in the low dword, entry1 in the high dword).
///
/// Fields, all from `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:266-284` and the
/// same four [`gp_entry`] writes.
///
/// Returns `None` for an entry that names **no method words**: `LENGTH == 0`. See
/// [`GpfifoEntry`] for why that is a refusal and not a zero-length range.
#[must_use]
pub const fn gp_entry_decode(entry: u64) -> Option<GpfifoEntry> {
    let entry0 = entry & 0xFFFF_FFFF;
    let entry1 = entry >> 32;
    // GP_ENTRY1_LENGTH is 30:10, in dwords.
    let dwords = (entry1 >> 10) & 0x1F_FFFF;
    if dwords == 0 {
        return None;
    }
    // GP_ENTRY0_GET is 31:2 — the address' low 32 bits with bits 1:0 forced to zero,
    // because bit 0 is FETCH and bit 1 is unused.
    let lo = entry0 & 0xFFFF_FFFC;
    // GP_ENTRY1_GET_HI is 7:0 — address bits 39:32.
    let hi = entry1 & 0xFF;
    Some(GpfifoEntry {
        gpu_va: lo | (hi << 32),
        len_bytes: dwords * 4,
        subroutine: (entry1 >> 9) & 1 == 1,
        sync_wait: (entry1 >> 31) & 1 == 1,
    })
}

/// `AMPERE_USERMODE_A` — re-exported from [`crate::generated::classes`].
///
/// ★ It was a hand-written literal here until `#156`. It is generated now, for one
/// reason: its Hopper counterpart `HOPPER_USERMODE_A` had to be pinned against the
/// vendored headers, and two halves of one seam checked by two different mechanisms is
/// where a transcription typo survives. This alias stays so that `submit`'s doorbell
/// vocabulary is still readable in one place.
pub use crate::generated::classes::AMPERE_USERMODE_A;

/// `NVC361_NV_USERMODE__SIZE` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:31`. The BAR window the usermode
/// object exposes, in bytes.
pub const USERMODE_WINDOW_SIZE: u64 = 65536;

/// `NVC361_NOTIFY_CHANNEL_PENDING` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:33`.
///
/// ★★★ **The doorbell.** A 32-bit store of a channel's work-submit token to this offset
/// in the mapped usermode window tells the host unit that the named channel has work. It
/// is *not* an ioctl and there is no ioctl that can stand in for it — which is why
/// `kayfabe_isolate::RmBackend::ring_doorbell` was a named refusal until a mapping existed.
pub const USERMODE_NOTIFY_CHANNEL_PENDING: u64 = 0x0000_0090;

/// `NVC361_TIME_0` — `ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:31`.
///
/// ★★★ **The host GPU's free-running nanosecond counter, low word**, mirrored into the
/// usermode window. Read-only in hardware (`NV_VIRTUAL_FUNCTION_TIME_0` is `R--4R`,
/// `ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_vm.h:127`).
///
/// ★★★ **It is sixteen bytes from the doorbell** ([`USERMODE_NOTIFY_CHANNEL_PENDING`] at
/// `0x90`), so the two live in the SAME 4 KiB page and no page-granular mechanism — KVM
/// memslot, PTE, EPT entry — can separate them. That is not an inconvenience; it is the
/// reason `#128`'s answer is *read-native, write-trap* on one shared page rather than
/// *pass through the timer page and trap the doorbell page*. See
/// `docs/design/read_native_timer.md`.
pub const USERMODE_TIME_0: u64 = 0x0000_0080;

/// `NVC361_TIME_1` — `ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:32`. The high
/// word of [`USERMODE_TIME_0`]'s counter; `NV_PTIMER_TIME_1_NSEC` is `28:0`, so the top
/// three bits are not part of the value
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_timer.h:42`).
pub const USERMODE_TIME_1: u64 = 0x0000_0084;

/// `NV_PTIMER_TIME_1_NSEC` is `28:0` — the high word carries 29 significant bits, so the
/// assembled counter is 61 bits wide
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_timer.h:42`).
pub const PTIMER_TIME_1_NSEC_MASK: u32 = 0x1fff_ffff;

/// Assemble a PTIMER reading from a `(hi, lo)` pair, applying the 29-bit mask the
/// hardware field definition demands.
///
/// ⊘ The caller owes the *sampling* discipline (read hi, lo, hi again and retry on a
/// carry); this function only says how the two words compose. Splitting it out is what
/// lets the composition be tested without a GPU.
#[must_use]
pub const fn ptimer_compose(hi: u32, lo: u32) -> u64 {
    (((hi & PTIMER_TIME_1_NSEC_MASK) as u64) << 32) | (lo as u64)
}

/// How many `hi, lo, hi` rounds [`ptimer_sample`] will take before giving up.
///
/// The low word wraps every 2^32 ns ≈ **4.29 seconds**, so two consecutive rounds both
/// straddling a carry is not a thing that happens on working hardware; a run of them means
/// the reads are not reading a counter. Three is a bound that surfaces that as a refusal
/// rather than as a hang — ⊘ never as a plausible constant.
pub const PTIMER_SAMPLE_ROUNDS: usize = 3;

/// The reason a [`ptimer_sample`] could not produce a coherent reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtimerSampleError<E> {
    /// A read of one of the two words failed; the transport's own error is carried.
    Read(E),
    /// [`PTIMER_SAMPLE_ROUNDS`] rounds all straddled a carry.
    ///
    /// ★ This is the refusal the standing rule demands. The tempting alternative — return
    /// the last `(hi, lo)` anyway — is a *plausible* answer off by up to 4.29 s, and a
    /// plausible wrong time is exactly what nobody can debug.
    Incoherent,
}

/// Read a 64-bit PTIMER value out of a pair of 32-bit registers, coherently.
///
/// ★★★ **Two 32-bit reads of a free-running counter are not one 64-bit read.** Between
/// them the low word can wrap and carry into the high word, and the naive `(hi, lo)` pair
/// is then ~4.29 s in the future or the past. So: read `hi`, read `lo`, read `hi` again,
/// and accept only when the two `hi` readings agree — the standard non-atomic
/// wide-counter protocol, and the same one RM itself uses
/// (`ogkm-580: src/nvidia/src/kernel/gpu/timer/arch/maxwell/timer_gm107.c:100-130`).
///
/// `read` is given a byte offset into whatever carries the pair, so the same code serves
/// the PTIMER page (`0x400`/`0x410` inside [`PTIMER_PAGE_SIZE`]) and the usermode mirror
/// ([`USERMODE_TIME_0`]/[`USERMODE_TIME_1`]), which are the same counter at two addresses.
///
/// # Errors
/// [`PtimerSampleError::Read`] if the transport refused, [`PtimerSampleError::Incoherent`]
/// if every round straddled a carry.
pub fn ptimer_sample<E>(
    hi_offset: u64,
    lo_offset: u64,
    mut read: impl FnMut(u64) -> Result<u32, E>,
) -> Result<u64, PtimerSampleError<E>> {
    for _ in 0..PTIMER_SAMPLE_ROUNDS {
        let hi = read(hi_offset).map_err(PtimerSampleError::Read)?;
        let lo = read(lo_offset).map_err(PtimerSampleError::Read)?;
        let hi_again = read(hi_offset).map_err(PtimerSampleError::Read)?;
        if hi == hi_again {
            return Ok(ptimer_compose(hi, lo));
        }
    }
    Err(PtimerSampleError::Incoherent)
}

/// `DRF_BASE(NV_PTIMER)` — `ogkm-580:
/// src/common/inc/swref/published/turing/tu104/dev_timer.h:26` (`0x00009fff:0x00009000`),
/// and what `tmrGetTimerBar0MapInfo_PTIMER` reports as the mappable timer range
/// (`ogkm-580: src/nvidia/src/kernel/gpu/timer/timer_ptimer.c:176-187`).
///
/// ★★★ **MEASURED**, not transcribed: `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` on a
/// real GA106 (RTX 3060, host driver 580.159.04) answers `NV_OK` with exactly this value,
/// **as an unprivileged uid as well as as root** — see
/// `docs/reference/bench_evidence/timer-mappability-*.out`.
pub const PTIMER_BAR0_BASE: u64 = 0x0000_9000;

/// `DRF_SIZE(NV_PTIMER)` — one 4 KiB page, and the whole of it is timer.
///
/// ★★★ This is the property that makes `#128` expressible at all: the PTIMER range is
/// **exactly one page and contains no doorbell**, so a page-granular read-native mapping
/// of it exposes the counter and nothing else. Contrast [`USERMODE_TIME_0`], which is 16
/// bytes from [`USERMODE_NOTIFY_CHANNEL_PENDING`].
pub const PTIMER_PAGE_SIZE: u64 = 0x1000;

/// `NV_PTIMER_TIME_0` relative to [`PTIMER_BAR0_BASE`] — `0x9400 - 0x9000`
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_timer.h:26`). Matches
/// `Nv01TimerMap::PTimerTime0`, which sits after `Reserved00[0x100]`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/cl0004.h:39-44`).
pub const PTIMER_PAGE_TIME_0: u64 = 0x0000_0400;

/// `NV_PTIMER_TIME_1` relative to [`PTIMER_BAR0_BASE`] — `0x9410 - 0x9000`
/// (`ogkm-580: src/common/inc/swref/published/maxwell/gm107/dev_timer.h:27`).
pub const PTIMER_PAGE_TIME_1: u64 = 0x0000_0410;

/// `sizeof(Nv01TimerMap)` — the length `tmrapiGetRegBaseOffsetAndSize_IMPL` reports for an
/// [`NV01_TIMER`] object (`ogkm-580: src/nvidia/src/kernel/gpu/timer/timer.c:1712-1734`,
/// struct at `src/common/sdk/nvidia/inc/class/cl0004.h:39-44`):
/// `Reserved00[0x100]` + `TIME_0` + `Reserved01[3]` + `TIME_1` = `0x414`.
pub const NV01_TIMER_MAP_SIZE: u64 = 0x414;

/// `NV01_TIMER` — `ogkm-580: src/common/sdk/nvidia/inc/class/cl0004.h:32`.
///
/// ⚠ **Hand-written, not generated.** `kayfabe-abi-gen` emits alloc-parameter structs and
/// the class ids that select them; `NV01_TIMER` takes **no** alloc parameters (
/// `tmrapiConstruct_IMPL` ignores them, `ogkm-580: .../timer/timer.c:1692-1701`), so the
/// generator has nothing to hang it off. The citation above is the whole of its sourcing.
pub const NV01_TIMER: u32 = 0x0000_0004;

/// `NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET` —
/// `ogkm-580: src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080tmr.h:119`. Answers
/// `NV2080_CTRL_TIMER_GET_REGISTER_OFFSET_PARAMS { NvU32 tmr_offset; }`, four bytes.
pub const NV2080_CTRL_CMD_TIMER_GET_REGISTER_OFFSET: u32 = 0x2080_0404;

// =====================================================================================
// Pushbuffer method encoding
// =====================================================================================

/// The number of method-address bits in a pushbuffer header — `NVC56F_DMA_INCR_ADDRESS`
/// is `11:0` (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:308`).
const METHOD_ADDRESS_BITS: u32 = 12;

/// The number of methods one header may carry — `NVC56F_DMA_INCR_COUNT` is `28:16`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:310`).
const METHOD_COUNT_BITS: u32 = 13;

/// The number of subchannels — `NVC56F_NUMBER_OF_SUBCHANNELS`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:67`).
pub const NUMBER_OF_SUBCHANNELS: u32 = 8;

/// Build an **incrementing** pushbuffer method header:
/// `count` dwords of data follow, applied to consecutive methods starting at `method`.
///
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:307-312` —
/// `ADDRESS` `11:0`, `SUBCHANNEL` `15:13`, `COUNT` `28:16`, `OPCODE` `31:29` = 1.
///
/// ★ `method` is the **byte** offset from the class header (e.g. `NVC7B5_OFFSET_IN_UPPER`
/// = `0x400`) and the field holds it **shifted right by two**: methods are dword-indexed.
/// Passing the byte offset unshifted names a method 4× further along, which on a copy
/// engine is a completely different register and does not fault.
///
/// Returns `None` rather than a truncated header when any field overflows.
#[must_use]
pub const fn method_header_inc(subchannel: u32, method: u32, count: u32) -> Option<u32> {
    if !method.is_multiple_of(4) {
        return None;
    }
    let addr = method / 4;
    if addr >= 1 << METHOD_ADDRESS_BITS {
        return None;
    }
    if subchannel >= NUMBER_OF_SUBCHANNELS {
        return None;
    }
    if count == 0 || count >= 1 << METHOD_COUNT_BITS {
        return None;
    }
    // OPCODE_VALUE = 1 at bits 31:29 — `clc56f.h:311-312`.
    Some(addr | (subchannel << 13) | (count << 16) | (1 << 29))
}

/// ★★★ The `NVC56F_DMA_SEC_OP` universe — **all eight**, as the class header enumerates
/// them (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:301-308`).
///
/// ★ It is a module of named constants plus [`sec_op::ALL`] for one reason recorded in
/// `gates_quantified_over_a_list`: a decoder's coverage claim has to be quantified over
/// the *driver's* enumeration, not over a list the decoder's author remembered. A codec
/// that handles six of these looks identical to one that handles eight until something
/// quantifies. `tests/tests/pushbuffer_abi_oracle.rs` reads the enumeration back out of
/// NVIDIA's own header and compares it against [`sec_op::ALL`], so a ninth opcode in a
/// later release turns that test red rather than being silently outside the universe.
pub mod sec_op {
    /// `NVC56F_DMA_SEC_OP_GRP0_USE_TERT` — the legacy group-0 encoding; `TERT_OP`
    /// (`17:16`) selects between a legacy incrementing method and the sub-device-mask
    /// operations.
    pub const GRP0_USE_TERT: u32 = 0;
    /// `NVC56F_DMA_SEC_OP_INC_METHOD` — `count` dwords applied to consecutive methods.
    /// This is the one [`super::method_header_inc`] writes and the one everything in this
    /// module's own pushbuffers uses.
    pub const INC_METHOD: u32 = 1;
    /// `NVC56F_DMA_SEC_OP_GRP2_USE_TERT` — the legacy group-2 encoding; `TERT_OP == 0` is
    /// a legacy non-incrementing method and no other `TERT_OP` value is enumerated.
    pub const GRP2_USE_TERT: u32 = 2;
    /// `NVC56F_DMA_SEC_OP_NON_INC_METHOD` — `count` dwords all applied to one method.
    pub const NON_INC_METHOD: u32 = 3;
    /// `NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD` — the datum is in the header itself
    /// (`NVC56F_DMA_IMMD_DATA`, `28:16`), so **no** words follow.
    pub const IMMD_DATA_METHOD: u32 = 4;
    /// `NVC56F_DMA_SEC_OP_ONE_INC` — the first dword increments, the rest do not.
    pub const ONE_INC: u32 = 5;
    /// ⊘ `NVC56F_DMA_SEC_OP_RESERVED6` — enumerated by the header **with no format**.
    /// [`super::method_header_decode`] returns `None` for it: it is the one opcode whose
    /// argument count the class header does not define, and inventing one is how a parser
    /// desynchronises onto data.
    pub const RESERVED6: u32 = 6;
    /// `NVC56F_DMA_SEC_OP_END_PB_SEGMENT` — the segment ends here; no words follow.
    pub const END_PB_SEGMENT: u32 = 7;

    /// Every value above, in numeric order. See the module docs for why this exists.
    pub const ALL: [u32; 8] = [
        GRP0_USE_TERT,
        INC_METHOD,
        GRP2_USE_TERT,
        NON_INC_METHOD,
        IMMD_DATA_METHOD,
        ONE_INC,
        RESERVED6,
        END_PB_SEGMENT,
    ];
}

/// `NVC56F_DMA_TERT_OP_GRP0_INC_METHOD` / `..._GRP2_NON_INC_METHOD` — both `0`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:294-299`). The other three
/// `TERT_OP` values are the sub-device-mask operations and exist only under
/// [`sec_op::GRP0_USE_TERT`].
const TERT_OP_METHOD: u32 = 0;

/// How a pushbuffer header says its arguments are applied.
///
/// ⚠ **This is a statement about the ARGUMENT STREAM, not about meaning.** It says how
/// many words follow and how they are distributed over method addresses; what those words
/// *are* is the [`Arch`](../../kayfabe_arch/trait.Arch.html)'s question, one crate up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodForm {
    /// [`sec_op::INC_METHOD`] — `arg_words` dwords at `method`, `method + 4`, ….
    Incrementing,
    /// [`sec_op::NON_INC_METHOD`] — `arg_words` dwords, all at `method`.
    NonIncrementing,
    /// [`sec_op::ONE_INC`] — the first dword at `method`, the rest at `method + 4`.
    IncrementOnce,
    /// [`sec_op::IMMD_DATA_METHOD`] — the datum is in [`MethodHeader::immd`]; no words
    /// follow.
    Immediate,
    /// [`sec_op::END_PB_SEGMENT`] — the segment ends; no words follow.
    EndPbSegment,
    /// The legacy [`sec_op::GRP0_USE_TERT`] / [`sec_op::GRP2_USE_TERT`] method formats,
    /// whose address is `NVC56F_DMA_METHOD_ADDRESS_OLD` (`12:2`) and whose count is
    /// `NVC56F_DMA_METHOD_COUNT_OLD` (`28:18`).
    ///
    /// ★★ **Sized, and only sized.** This port models none of these, so nothing decodes
    /// their arguments — but a header this parser cannot *size* makes it advance by one
    /// word and read every following datum as a header, which is how a stream of numbers
    /// becomes a stream of plausible methods. Sizing the legacy forms costs two lines and
    /// removes that door; refusing to size them would have left it open in the name of
    /// caution. ⊘ Note `NVC56F_DMA_NOP` is `0x00000000`, i.e. this form with a zero
    /// count — so a zero-filled pushbuffer parses as a run of NOPs and never desyncs.
    Legacy,
    /// The sub-device-mask operations ([`sec_op::GRP0_USE_TERT`] with a non-zero
    /// `TERT_OP`) — header-only, no words follow.
    SubDeviceMask,
}

/// One decoded pushbuffer header word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodHeader {
    /// How the arguments are applied.
    pub form: MethodForm,
    /// The **byte** offset of the first method addressed, i.e. the header's dword index
    /// multiplied by four — the same units [`method_header_inc`] takes and the units the
    /// class headers name their methods in (`NVC7B5_LAUNCH_DMA` = `0x300`).
    pub method: u32,
    /// `SUBCHANNEL` (`15:13`).
    pub subchannel: u32,
    /// How many dwords follow this header. Zero for
    /// [`MethodForm::Immediate`]/[`MethodForm::EndPbSegment`]/[`MethodForm::SubDeviceMask`].
    pub arg_words: usize,
    /// `NVC56F_DMA_IMMD_DATA` (`28:16`), meaningful only for [`MethodForm::Immediate`].
    pub immd: u32,
}

/// Decode one pushbuffer header word — the read side of [`method_header_inc`], widened to
/// every format `NVC56F_DMA_SEC_OP` enumerates.
///
/// ⊘ Returns `None` for the two encodings the class header does **not** define:
/// [`sec_op::RESERVED6`], and [`sec_op::GRP2_USE_TERT`] with a `TERT_OP` other than
/// `GRP2_NON_INC_METHOD`. Those are the only header words on this chip whose argument
/// count is not a fact, and a guessed count is a parser walking off its own stream.
#[must_use]
pub const fn method_header_decode(header: u32) -> Option<MethodHeader> {
    let sec_op = header >> 29;
    let tert_op = (header >> 16) & 0x3;
    let subchannel = (header >> 13) & 0x7;
    // NVC56F_DMA_METHOD_ADDRESS is 11:0, dword-indexed.
    let method = (header & 0xFFF) * 4;
    // NVC56F_DMA_METHOD_COUNT is 28:16.
    let count = ((header >> 16) & 0x1FFF) as usize;
    // The legacy pair: NVC56F_DMA_METHOD_ADDRESS_OLD is 12:2, COUNT_OLD is 28:18.
    let old_method = ((header >> 2) & 0x7FF) * 4;
    let old_count = ((header >> 18) & 0x7FF) as usize;

    let (form, method, arg_words) = match sec_op {
        sec_op::INC_METHOD => (MethodForm::Incrementing, method, count),
        sec_op::NON_INC_METHOD => (MethodForm::NonIncrementing, method, count),
        sec_op::ONE_INC => (MethodForm::IncrementOnce, method, count),
        sec_op::IMMD_DATA_METHOD => (MethodForm::Immediate, method, 0),
        sec_op::END_PB_SEGMENT => (MethodForm::EndPbSegment, method, 0),
        sec_op::GRP0_USE_TERT if tert_op == TERT_OP_METHOD => {
            (MethodForm::Legacy, old_method, old_count)
        }
        sec_op::GRP0_USE_TERT => (MethodForm::SubDeviceMask, 0, 0),
        sec_op::GRP2_USE_TERT if tert_op == TERT_OP_METHOD => {
            (MethodForm::Legacy, old_method, old_count)
        }
        // GRP2 with an unenumerated TERT_OP, and RESERVED6. No format, no size.
        _ => return None,
    };
    Some(MethodHeader {
        form,
        method,
        subchannel,
        arg_words,
        immd: (header >> 16) & 0x1FFF,
    })
}

/// `NVC56F_SET_OBJECT` — `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:68`.
///
/// The method that binds a class to a subchannel. It is the first thing a pushbuffer
/// carrying *engine* methods must do, and one that omits it addresses whatever the
/// subchannel last held. ★ It is **not** needed for the host-FIFO methods in [`fifo`]:
/// those are executed by the channel's own front end, not by an engine, which is exactly
/// what makes them provable with no engine object allocated.
pub const SET_OBJECT: u32 = 0x0000_0000;

/// ★★★ The **host-FIFO semaphore** methods — `NVC56F_SEM_*`,
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:206-231`.
///
/// These are the smallest thing a GPU can be asked to do that leaves *evidence*: five
/// consecutive methods that make the channel's front end write a chosen payload to a
/// chosen GPU virtual address. No engine class object, no golden context, no compute —
/// so a failure localises to the submission machinery itself and to nothing else.
///
/// That is why the C's host channel self-test uses exactly this and nothing more
/// (`C: src/qemu/nvkvm_gpu_emul.c:8595-8604`, `:9595-9639`), on a copy-engine runlist
/// deliberately chosen because it has no graphics-context dependency. Its verdict line
/// is the bar this port is trying to clear: *"SEM LANDED — host doorbell+schedule+USERD
/// mechanics GOOD"*.
pub mod fifo {
    /// `NVC56F_SEM_ADDR_LO` @ `0x5c` — semaphore address bits `31:2`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:206-207`).
    pub const SEM_ADDR_LO: u32 = 0x0000_005C;
    /// `NVC56F_SEM_ADDR_HI` @ `0x60` — semaphore address bits `39:32`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:208-209`). **Eight bits**:
    /// the same 2^40 ceiling [`super::gp_entry`] enforces.
    pub const SEM_ADDR_HI: u32 = 0x0000_0060;
    /// `NVC56F_SEM_PAYLOAD_LO` @ `0x64`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:210-211`).
    pub const SEM_PAYLOAD_LO: u32 = 0x0000_0064;
    /// `NVC56F_SEM_PAYLOAD_HI` @ `0x68`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:212-213`).
    pub const SEM_PAYLOAD_HI: u32 = 0x0000_0068;
    /// `NVC56F_SEM_EXECUTE` @ `0x6c`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:214-234`).
    pub const SEM_EXECUTE: u32 = 0x0000_006C;

    /// `SEM_EXECUTE_OPERATION_RELEASE` — field `2:0`, value 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:217`).
    ///
    /// ★ With `PAYLOAD_SIZE` (`24:24`) left `_32BIT` (0, `:230`), `RELEASE_WFI` left
    /// `_DIS` (0, `:227`) and `RELEASE_TIMESTAMP` left `_DIS` (0, `:233`), the whole
    /// method word is `1` — which is what the C writes (`C: nvkvm_gpu_emul.c:8603`). The
    /// value is spelled as a named constant rather than as a literal `1` because the
    /// three zeros are *choices*: a 64-bit payload writes eight bytes over a four-byte
    /// sentinel, and a timestamp release writes a 16-byte structure.
    pub const SEM_EXECUTE_RELEASE_32BIT: u32 = 1;

    /// `NVC56F_SEM_EXECUTE_OPERATION` is `2:0`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:215`) — the mask a *reader*
    /// needs, because seven other operations share the word and six of them are
    /// **acquires**. ★ Decoding an acquire as a release would report a completion the
    /// guest is still waiting for.
    pub const SEM_EXECUTE_OPERATION_MASK: u32 = 0x7;
    /// `NVC56F_SEM_EXECUTE_OPERATION_RELEASE` = 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:217`).
    pub const SEM_EXECUTE_OPERATION_RELEASE: u32 = 1;
    /// `NVC56F_SEM_EXECUTE_PAYLOAD_SIZE` is `24:24`, `_64BIT` = 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:229-231`). With `_32BIT`
    /// (0) the engine writes four bytes and `SEM_PAYLOAD_HI` is **not** part of the
    /// value — reading it anyway invents the top 32 bits of a fence.
    pub const SEM_EXECUTE_PAYLOAD_SIZE_64BIT: u32 = 1 << 24;

    /// `NVC56F_MEM_OP_A` @ `0x28`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:119`). ★ The header's own
    /// comment on `MEM_OP_D` is *"MEM_OP_D MUST be preceded by MEM_OPs A-C"*, which is
    /// why the only shape this port decodes is one incrementing run of **four** starting
    /// here — the four words are one fact and no prefix of them is.
    pub const MEM_OP_A: u32 = 0x0000_0028;
    /// `NVC56F_MEM_OP_A_TLB_INVALIDATE_SYSMEMBAR` is `11:11`, `_EN` = 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:129-130`).
    pub const MEM_OP_A_SYSMEMBAR_EN: u32 = 1 << 11;
    /// `NVC56F_MEM_OP_C_TLB_INVALIDATE_PDB` is `0:0`, `_ONE` = 0
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:139-141`). `_ALL` carries no
    /// PDB address at all, so the address fields below are meaningless for it.
    pub const MEM_OP_C_PDB_ALL: u32 = 1;
    /// `NVC56F_MEM_OP_C_TLB_INVALIDATE_PDB_ADDR_LO` is `31:12`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:178`).
    pub const MEM_OP_C_PDB_ADDR_LO_MASK: u32 = 0xFFFF_F000;
    /// `NVC56F_MEM_OP_D_TLB_INVALIDATE_PDB_ADDR_HI` is `26:0`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:182`) — address bits `58:32`.
    pub const MEM_OP_D_PDB_ADDR_HI_MASK: u32 = 0x07FF_FFFF;
    /// `NVC56F_MEM_OP_D_OPERATION` is `31:27`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:183`).
    pub const MEM_OP_D_OPERATION_SHIFT: u32 = 27;
    /// `NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE` = 9
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:185`).
    pub const MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE: u32 = 9;
    /// `NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED` = 0xa
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:186`).
    pub const MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED: u32 = 0xa;
}

/// `NVC56F_SET_OBJECT_NVCLASS` is `15:0`
/// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:69`) — the class id a
/// `SET_OBJECT` binds. ★ Bits `20:16` are `ENGINE`, and a decoder that took the whole
/// dword would report a class id nothing in `kayfabe_abi::generated::classes` names,
/// turning every subchannel bind on a non-zero engine into an unknown class.
pub const SET_OBJECT_NVCLASS_MASK: u32 = 0xFFFF;

// =====================================================================================
// The copy engine's methods
// =====================================================================================

/// `AMPERE_DMA_COPY_B` method offsets —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h`.
///
/// A module of constants rather than an enum: these are *addresses in a method space*,
/// they are consumed by arithmetic ([`method_header_inc`]), and an enum would invite a
/// discriminant cast — the exact shape of the wire-code bug
/// `kayfabe_isolate_host::proto::engine_code` exists to prevent.
pub mod ce {
    /// `NVC7B5_SET_SEMAPHORE_A` @ `0x240` — the semaphore address, bits `48:32`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:47-48`).
    pub const SET_SEMAPHORE_A: u32 = 0x0000_0240;
    /// `NVC7B5_SET_SEMAPHORE_B` @ `0x244` — the semaphore address, bits `31:0`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:49-50`).
    pub const SET_SEMAPHORE_B: u32 = 0x0000_0244;
    /// `NVC7B5_SET_SEMAPHORE_PAYLOAD` @ `0x248` — the value the engine will write
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:51-52`).
    pub const SET_SEMAPHORE_PAYLOAD: u32 = 0x0000_0248;
    /// `NVC7B5_LAUNCH_DMA` @ `0x300` — the method that starts the copy
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:84`).
    pub const LAUNCH_DMA: u32 = 0x0000_0300;
    /// `NVC7B5_OFFSET_IN_UPPER` @ `0x400`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:161`).
    ///
    /// ★ The four address methods are **consecutive** — `0x400`, `0x404`, `0x408`,
    /// `0x40c` — so one incrementing header of count 4 starting here writes the whole
    /// source-and-destination pair. That is why the two `_LOWER` constants exist next to
    /// their `_UPPER`s rather than being derived by adding four at a call site.
    pub const OFFSET_IN_UPPER: u32 = 0x0000_0400;
    /// `NVC7B5_OFFSET_IN_LOWER` @ `0x404` — the source address' low 32 bits
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:163`).
    pub const OFFSET_IN_LOWER: u32 = 0x0000_0404;
    /// `NVC7B5_OFFSET_OUT_UPPER` @ `0x408`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:165`).
    pub const OFFSET_OUT_UPPER: u32 = 0x0000_0408;
    /// `NVC7B5_OFFSET_OUT_LOWER` @ `0x40c` — the destination address' low 32 bits
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:167`).
    pub const OFFSET_OUT_LOWER: u32 = 0x0000_040C;
    /// `NVC7B5_LINE_LENGTH_IN` @ `0x418`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:173`).
    pub const LINE_LENGTH_IN: u32 = 0x0000_0418;
    /// `NVC7B5_LINE_COUNT` @ `0x41C`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:175`).
    pub const LINE_COUNT: u32 = 0x0000_041C;

    /// `LAUNCH_DMA_DATA_TRANSFER_TYPE_NON_PIPELINED` — field `1:0`, value 2
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:88`).
    pub const LAUNCH_TRANSFER_NON_PIPELINED: u32 = 2;
    /// `LAUNCH_DMA_FLUSH_ENABLE_TRUE` — field `2:2`, value 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:91`).
    pub const LAUNCH_FLUSH_ENABLE: u32 = 1 << 2;
    /// `LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_ONE_WORD_SEMAPHORE` — field `4:3`, value 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:99`).
    ///
    /// ★ The whole reason a CE copy is *provable*: the engine writes the payload to the
    /// semaphore address **after** the copy retires. A payload that appears is the
    /// hardware saying it finished, and it is the only signal in this module that our own
    /// code cannot produce.
    pub const LAUNCH_SEMAPHORE_RELEASE_ONE_WORD: u32 = 1 << 3;
    /// `SET_SEMAPHORE_A_UPPER` — field `16:0` (`clc7b5.h:48`), i.e. semaphore address bits
    /// `48:32`. ⊘ Masked rather than shifted whole: bits above `16:0` are not part of the
    /// address, and carrying them would move a semaphore into a page nothing mapped.
    pub const SET_SEMAPHORE_A_UPPER_MASK: u32 = 0x0001_FFFF;
    /// `LAUNCH_DMA_SEMAPHORE_TYPE` — field `4:3` (`clc7b5.h:96`).
    pub const LAUNCH_SEMAPHORE_TYPE_MASK: u32 = 0x3 << 3;
    /// `LAUNCH_DMA_SEMAPHORE_TYPE_NONE` — value 0 (`clc7b5.h:97`). The launch releases
    /// nothing; the `SET_SEMAPHORE_*` registers are not read at all.
    pub const LAUNCH_SEMAPHORE_TYPE_NONE: u32 = 0;
    /// `LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_FOUR_WORD_SEMAPHORE` — value 2, i.e. the
    /// with-timestamp release (`clc7b5.h:100, :102`). Sixteen bytes: the payload plus a
    /// **hardware timestamp** this port has no source for.
    pub const LAUNCH_SEMAPHORE_TYPE_RELEASE_FOUR_WORD: u32 = 2 << 3;
    /// `LAUNCH_DMA_SEMAPHORE_TYPE_RELEASE_CONDITIONAL_INTR_SEMAPHORE` — value 3
    /// (`clc7b5.h:103`).
    pub const LAUNCH_SEMAPHORE_TYPE_RELEASE_CONDITIONAL_INTR: u32 = 3 << 3;
    /// `LAUNCH_DMA_SRC_MEMORY_LAYOUT_PITCH` — field `7:7`, value 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:108`).
    pub const LAUNCH_SRC_PITCH: u32 = 1 << 7;
    /// `LAUNCH_DMA_DST_MEMORY_LAYOUT_PITCH` — field `8:8`, value 1
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:111`).
    pub const LAUNCH_DST_PITCH: u32 = 1 << 8;
    /// `LAUNCH_DMA_MULTI_LINE_ENABLE_FALSE` — field `9:9`, value 0
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:113`). Named, not omitted: a
    /// single-line copy is a *choice* about how `LINE_LENGTH_IN` is read, and leaving it
    /// implicit is how a byte count becomes a pitch.
    pub const LAUNCH_MULTI_LINE_DISABLE: u32 = 0;
    /// `LAUNCH_DMA_SRC_TYPE_VIRTUAL` — field `12:12`, value 0
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:122`).
    ///
    /// ★★ Virtual, always, for anything an isolate submits. `_PHYSICAL` points the engine
    /// at physical addresses with no MMU between it and the rest of the machine; nothing
    /// in this project's threat model permits it, and the constant is deliberately absent
    /// rather than present-and-unused.
    pub const LAUNCH_SRC_VIRTUAL: u32 = 0;
    /// `LAUNCH_DMA_DST_TYPE_VIRTUAL` — field `13:13`, value 0
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:125`).
    pub const LAUNCH_DST_VIRTUAL: u32 = 0;

    // ---------------------------------------------------------------------------------
    // ★ E5 — the DECODE side. Everything above is what an encoder ORs in; a decoder needs
    // the field extents, and reading a set bit as a field is how a decoder invents one.
    // ---------------------------------------------------------------------------------

    /// `LAUNCH_DMA_DATA_TRANSFER_TYPE` — field `1:0`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:85`).
    pub const LAUNCH_TRANSFER_MASK: u32 = 0x3;
    /// `LAUNCH_DMA_DATA_TRANSFER_TYPE_NONE` — value 0 (`clc7b5.h:86`). ⊘ A launch with
    /// this moves **no bytes**; it exists to release a semaphore. Decoding one as a copy
    /// would report a transfer the engine never performs.
    pub const LAUNCH_TRANSFER_NONE: u32 = 0;
    /// `LAUNCH_DMA_MULTI_LINE_ENABLE_TRUE` — field `9:9`, value 1 (`clc7b5.h:112-114`).
    pub const LAUNCH_MULTI_LINE_ENABLE: u32 = 1 << 9;
    /// `LAUNCH_DMA_REMAP_ENABLE_TRUE` — field `10:10`, value 1 (`clc7b5.h:115-117`).
    pub const LAUNCH_REMAP_ENABLE: u32 = 1 << 10;
    /// `LAUNCH_DMA_SRC_TYPE_PHYSICAL` — field `12:12`, value 1 (`clc7b5.h:123`).
    pub const LAUNCH_SRC_PHYSICAL: u32 = 1 << 12;
    /// `LAUNCH_DMA_DST_TYPE_PHYSICAL` — field `13:13`, value 1 (`clc7b5.h:126`).
    pub const LAUNCH_DST_PHYSICAL: u32 = 1 << 13;

    /// ★★★ **The subchannel hardware fixes for the copy engine** — `4`.
    ///
    /// `[src] ogkm-580: src/common/sdk/nvidia/inc/class/cla06fsubch.h:30`
    /// (`NVA06F_SUBCHANNEL_COPY_ENGINE 4`), one of five architecturally assigned
    /// subchannels in that header — `3D 0`, `COMPUTE 1`, `I2M 2`, `2D 3`, `COPY_ENGINE 4`.
    ///
    /// # ⊘ Why a decoder needs it, and why it is NOT a licence to decode anything
    ///
    /// A subchannel's methods mean whatever its bound object says. `SET_OBJECT` is the
    /// in-band way to bind one — but it is not the only way the binding exists, and UVM
    /// proves it: `uvm_hal_maxwell_ce_init` binds the CE class on subchannel **0** on
    /// purpose (*"instead of the recommended by HW subchannel 4 … subchannel 4 is required
    /// to match CE usage on GRCE"*, `kernel-open/nvidia-uvm/uvm_maxwell_ce.c:31-35`) and
    /// then issues every CE method on subchannel 4, which nothing in the stream ever binds.
    ///
    /// ⇒ A codec that decodes an **unbound** subchannel's methods needs one narrow,
    /// **sourced** reason to believe what they are, and this is it: *this* subchannel, and
    /// no other. ⚠ It is a necessary condition, never a sufficient one — see
    /// [`kayfabe_arch::MethodState::subchannel_speaks`], which additionally requires that
    /// the guest named the class somewhere on the same channel.
    ///
    /// ⊘ The wider rule — *"any unbound subchannel, if the channel bound a CE anywhere"* —
    /// was written first and **a hostile-stream property refuted it in the same hour**
    /// (`tests/tests/pushbuffer_ga10x_hostile.rs`,
    /// `a_hostile_method_stream_never_fires_a_copy_it_did_not_write`): it fired a copy from
    /// **subchannel 6** carrying operands a compute object's method addresses could equally
    /// have written.
    pub const FIXED_SUBCHANNEL: usize = 4;

    /// `NVC7B5_SET_SRC_PHYS_MODE` method address (`ogkm-580: clc7b5.h:66`, and
    /// **identical** in `clb0b5.h:56`, so it is chip-family-stable across every
    /// `*_DMA_COPY_*` this port targets).
    pub const SET_SRC_PHYS_MODE: u32 = 0x0000_0260;
    /// `NVC7B5_SET_DST_PHYS_MODE` method address (`clc7b5.h:75` = `clb0b5.h:61`).
    pub const SET_DST_PHYS_MODE: u32 = 0x0000_0264;
    /// `SET_{SRC,DST}_PHYS_MODE_TARGET` — field `1:0` (`clc7b5.h:67, :76`). The residency
    /// of a physical operand; the enumerated values are `LOCAL_FB=0`, `COHERENT_SYSMEM=1`,
    /// `NONCOHERENT_SYSMEM=2`, `PEERMEM=3` (`clc7b5.h:68-71`), and the register resets to 0
    /// = `LOCAL_FB`.
    pub const PHYS_MODE_TARGET_MASK: u32 = 0x3;

    /// `OFFSET_IN_UPPER_UPPER` / `OFFSET_OUT_UPPER_UPPER` — **`16:0`**, i.e. seventeen
    /// bits, not thirty-two (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:162,
    /// :166`). A decoder that took the whole word would report a destination the engine
    /// cannot address, and one that took eight bits — the GPFIFO entry's width, which is
    /// the nearby number a reader is likely to reuse — would report a *different page*,
    /// silently.
    pub const OFFSET_UPPER_MASK: u32 = 0x1_FFFF;

    /// ★★★ **`MEMORY_SCRUB_ENABLE` DOES NOT EXIST ON THIS CLASS, and the C reads it
    /// anyway.**
    ///
    /// `[src]` `grep -c MEMORY_SCRUB ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h`
    /// is **0**. The field is `NVC8B5_LAUNCH_DMA_MEMORY_SCRUB_ENABLE` at `23:23`
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc8b5.h:84-86`) — a **Hopper** class.
    /// On `NVC7B5`, bit 23 is the top half of `VPRMODE` (`23:22`, `clc7b5.h:146-148`).
    ///
    /// ⊘ The C artifact reads `bool mscrub = (d >> 23) & 1; /* MEMORY_SCRUB_ENABLE [23] */`
    /// (`C: src/qemu/nvkvm_gpu_emul.c:6208`) and feeds it to its execute predicate at
    /// `:6310`. On the Ampere part the C actually ran, that conjunct is reading a video-
    /// protected-region mode, and since neither enumerated `VPRMODE` value sets bit 23 it
    /// is a constant `false` — so the C's scrub arm is **unreachable** on GA10x and its
    /// `!mscrub` conjunct is vacuous.
    ///
    /// ⇒ This port therefore cannot produce `CeWork::Scrub` from a GA10x `LAUNCH_DMA`, and
    /// the constant is deliberately **absent** rather than present-and-wrong. `port_the_c`
    /// says reproduce the C and subtract its named bugs; this is one of them, named.
    pub const NO_MEMORY_SCRUB_ON_THIS_CLASS: () = ();

    // ---------------------------------------------------------------------------------
    // ★★★ THE REMAP (constant-fill) REGISTERS — `SET_REMAP_CONST_A/_B` and the component
    // map that says what an element of the fill actually IS.
    //
    // ⚠⚠ **`SET_REMAP_COMPONENTS` is not decoration: it changes the meaning of TWO other
    // registers**, and a decoder that fires a remap-enabled launch without reading it gets
    // both of them wrong. `[src]` `ogkm-580:
    // kernel-open/nvidia-uvm/uvm_maxwell_ce.c:330-420` is the driver stating both, in
    // code that runs on this exact class:
    //
    // 1. **`LINE_LENGTH_IN` counts ELEMENTS, not bytes.** `uvm_hal_maxwell_ce_memset_4`
    //    does `size /= 4` before handing `size` to `memset_common`, which pushes
    //    `LINE_LENGTH_IN = size` and then advances `dst.address += memset_this_time *
    //    memset_element_size` (`:359`, `:371`, `:396-402`). An element is
    //    `COMPONENT_SIZE × NUM_DST_COMPONENTS` bytes.
    // 2. **The pattern's PERIOD is the element, not four bytes.** `memset_1` puts an
    //    `NvU8` in `CONST_B` with `COMPONENT_SIZE_ONE` (`:379-386`) — a **byte** fill;
    //    `memset_8` puts a 64-bit value across `CONST_A`+`CONST_B` with
    //    `NUM_DST_COMPONENTS_TWO` (`:407-419`) — an **8-byte** period.
    //
    // ★ And RM's own scrub/memset path is the 1-byte map: `channelPushMemoryProperties`
    // pushes `DST_X = CONST_A | COMPONENT_SIZE_ONE | NUM_DST_COMPONENTS_ONE` (`ogkm-580:
    // channel_utils.c:1029-1033`) — so `memmgrMemSet`'s CE arm writes `pattern & 0xFF` to
    // every byte, exactly as its own `TRANSFER_TYPE_PROCESSOR` arm's
    // `portMemSet(pDst, value, size)` does (`ogkm-580: mem_utils.c:1122`). The two arms of
    // one operation must be observationally equal, and that is the corroboration.

    /// `NVC7B5_SET_REMAP_CONST_A` @ `0x700` — the `A` constant a component map may select
    /// (`ogkm-580: src/common/sdk/nvidia/inc/class/clc7b5.h:177-178` = `clb0b5.h`'s
    /// `NVB0B5_SET_REMAP_CONST_A`, same address).
    pub const SET_REMAP_CONST_A: u32 = 0x0000_0700;
    /// `NVC7B5_SET_REMAP_CONST_B` @ `0x704` (`clc7b5.h:179-180`).
    pub const SET_REMAP_CONST_B: u32 = 0x0000_0704;
    /// `NVC7B5_SET_REMAP_COMPONENTS` @ `0x708` (`clc7b5.h:181`).
    pub const SET_REMAP_COMPONENTS: u32 = 0x0000_0708;

    /// The width of one `DST_{X,Y,Z,W}` selector field, in bits — the four are at `2:0`,
    /// `6:4`, `10:8` and `14:12` (`clc7b5.h:182, :190, :198, :206`), i.e. a **stride of
    /// four bits** with the top bit of each nibble unused.
    ///
    /// ⊘ Expressed as a stride rather than four constants because a decoder walks
    /// `0..NUM_DST_COMPONENTS`, and four separately-named shifts is how the fourth one
    /// ends up copy-pasted from the third.
    pub const REMAP_DST_SEL_STRIDE: u32 = 4;
    /// Mask of one `DST_*` selector (three bits).
    pub const REMAP_DST_SEL_MASK: u32 = 0x7;
    /// `SET_REMAP_COMPONENTS_DST_*_CONST_A` — value 4 (`clc7b5.h:187`).
    pub const REMAP_DST_SEL_CONST_A: u32 = 4;
    /// `SET_REMAP_COMPONENTS_DST_*_CONST_B` — value 5 (`clc7b5.h:188`).
    pub const REMAP_DST_SEL_CONST_B: u32 = 5;
    /// `SET_REMAP_COMPONENTS_DST_*_SRC_{X,Y,Z,W}` — values `0..=3` (`clc7b5.h:183-186`).
    ///
    /// ⊘ Named so a decoder can **refuse** them rather than fall through: a selector
    /// naming a SOURCE component is a remapped *copy* (a swizzle), not a constant fill, and
    /// there is no pattern to report for it. `_NO_WRITE` (6, `clc7b5.h:189`) is a third
    /// thing again — a component the engine skips — and folding either into "fill" would
    /// claim bytes were written that were not.
    pub const REMAP_DST_SEL_SRC_MAX: u32 = 3;
    /// `SET_REMAP_COMPONENTS_DST_*_NO_WRITE` — value 6 (`clc7b5.h:189`).
    pub const REMAP_DST_SEL_NO_WRITE: u32 = 6;

    /// `SET_REMAP_COMPONENTS_COMPONENT_SIZE` — field `17:16` (`clc7b5.h:214`).
    pub const REMAP_COMPONENT_SIZE_SHIFT: u32 = 16;
    /// Mask of `COMPONENT_SIZE`, after shifting.
    pub const REMAP_COMPONENT_SIZE_MASK: u32 = 0x3;
    /// `SET_REMAP_COMPONENTS_NUM_DST_COMPONENTS` — field `25:24` (`clc7b5.h:224`).
    pub const REMAP_NUM_DST_COMPONENTS_SHIFT: u32 = 24;
    /// Mask of `NUM_DST_COMPONENTS`, after shifting.
    pub const REMAP_NUM_DST_COMPONENTS_MASK: u32 = 0x3;

    /// Bytes per component, from the raw `COMPONENT_SIZE` field.
    ///
    /// ⚠ **`_ONE` is encoded as `0`**, `_TWO` as `1`, `_THREE` as `2`, `_FOUR` as `3`
    /// (`clc7b5.h:215-218`) — the field is *size minus one*, which is why every one of
    /// RM's and UVM's `DRF_DEF(…, _ONE)` terms contributes a literal zero and a decoder
    /// that read the field as the size would report a **zero-byte element**.
    #[must_use]
    pub const fn remap_component_bytes(components: u32) -> u32 {
        ((components >> REMAP_COMPONENT_SIZE_SHIFT) & REMAP_COMPONENT_SIZE_MASK) + 1
    }

    /// How many destination components an element has, from the raw
    /// `NUM_DST_COMPONENTS` field. Same *minus one* encoding as
    /// [`remap_component_bytes`] (`clc7b5.h:225-228`).
    #[must_use]
    pub const fn remap_num_dst_components(components: u32) -> u32 {
        ((components >> REMAP_NUM_DST_COMPONENTS_SHIFT) & REMAP_NUM_DST_COMPONENTS_MASK) + 1
    }

    /// The `DST_*` selector for destination component `c` (`0` = X … `3` = W).
    #[must_use]
    pub const fn remap_dst_sel(components: u32, c: u32) -> u32 {
        (components >> (REMAP_DST_SEL_STRIDE * c)) & REMAP_DST_SEL_MASK
    }
}

// =====================================================================================
// The copy engine's ALLOC parameters — eight bytes with the whole runlist bug in them
// =====================================================================================

/// `NVB0B5_ALLOCATION_PARAMETERS` —
/// `ogkm-580: src/common/sdk/nvidia/inc/class/clb0b5sw.h:50-53`. Eight bytes, and they
/// are the alloc params for **every** `*_DMA_COPY_*` class including
/// [`crate::generated::classes::AMPERE_DMA_COPY_B`].
///
/// ★★★ **This is the C's proven `engineType = 0` bug, in struct form.** A copy-engine
/// object allocated with these eight bytes missing (or zeroed) does not fail: RM reads
/// zeros, `pParamToEngDescFn` defaults to `ENG_COPY(0)`, the channel lands on the
/// **graphics** runlist, and the failure surfaces two steps later as
/// `GPFIFO_SCHEDULE → NV_ERR_NOT_READY` or, further away still, as `cuCtxCreate`
/// returning 401 (`C: src/abi/nvgpu.h:87-95`, `dma_copy_class_alloc_params`, seam audit
/// GR-1). Nothing between the alloc and the symptom says the word "engine".
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CeAllocParams {
    /// `NvU32 version` @ +0. See [`CeAllocParams::VERSION_1`].
    pub version: u32,
    /// `NvU32 engineType` @ +4 — under [`CeAllocParams::VERSION_1`] an
    /// `NV2080_ENGINE_TYPE_*` ordinal, i.e. the **same number** the channel group was
    /// allocated with ([`ENGINE_TYPE_COPY0`]). Under `VERSION_0` it would instead be a
    /// bare CE *instance* index, which is a different numbering — hence no default.
    pub engine_type: u32,
}

impl CeAllocParams {
    /// The C typedef name.
    pub const C_NAME: &'static str = "NVB0B5_ALLOCATION_PARAMETERS";
    /// `sizeof`.
    pub const SIZE: usize = 8;
    /// `alignof`.
    pub const ALIGN: usize = 4;
    /// `NVB0B5_ALLOCATION_PARAMETERS_VERSION_1` —
    /// `ogkm-580: src/common/sdk/nvidia/inc/class/clb0b5sw.h:46`, *"engineType as an
    /// `NV2080_ENGINE_TYPE` ordinal"*. Version 0 reinterprets the same field as a CE
    /// instance for 85B5/90B5 compatibility (`:40`), so the version is not cosmetic:
    /// it selects which namespace the other four bytes are in.
    pub const VERSION_1: u32 = 1;

    /// Encode into a little-endian image of at least [`Self::SIZE`] bytes.
    ///
    /// # Errors
    /// [`AbiError::Truncated`].
    pub fn encode_into(&self, bytes: &mut [u8]) -> Result<(), AbiError> {
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            0,
            &self.version.to_le_bytes(),
        )?;
        put(
            bytes,
            Self::C_NAME,
            Self::SIZE,
            4,
            &self.engine_type.to_le_bytes(),
        )
    }
}

/// Bounds-checked field write. Same helper as [`crate::bringup`]'s, private to each
/// module on purpose: a shared one would have to pick a home, and neither module is the
/// other's dependency.
fn put(
    bytes: &mut [u8],
    c_name: &'static str,
    need: usize,
    off: usize,
    src: &[u8],
) -> Result<(), AbiError> {
    let got = bytes.len();
    bytes
        .get_mut(off..off + src.len())
        .ok_or(AbiError::Truncated { c_name, need, got })?
        .copy_from_slice(src);
    Ok(())
}

// The transcriptions vs rustc, at COMPILE time — the same gate the generated structs get.
const _: () = {
    assert!(core::mem::size_of::<CeAllocParams>() == CeAllocParams::SIZE);
    assert!(core::mem::align_of::<CeAllocParams>() == CeAllocParams::ALIGN);
    assert!(core::mem::offset_of!(CeAllocParams, version) == 0);
    assert!(core::mem::offset_of!(CeAllocParams, engine_type) == 4);
    assert!(core::mem::size_of::<Nvos33ParametersWithFd>() == Nvos33ParametersWithFd::SIZE);
    assert!(core::mem::align_of::<Nvos33ParametersWithFd>() == Nvos33ParametersWithFd::ALIGN);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, h_client) == 0);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, h_device) == 4);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, h_memory) == 8);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, offset) == 16);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, length) == 24);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, p_linear_address) == 32);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, status) == 40);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, flags) == 44);
    assert!(core::mem::offset_of!(Nvos33ParametersWithFd, fd) == 48);

    // `NvMemoryAllocationParams` has no `#[repr(C)]` mirror to take offsets of (like
    // `ChannelAllocParams`, it is an offset-addressed encoder over a struct whose tail we
    // do not model), so its layout is asserted by the wire test below instead.
    assert!(core::mem::size_of::<GpfifoScheduleParams>() == GpfifoScheduleParams::SIZE);
    assert!(core::mem::offset_of!(GpfifoScheduleParams, b_enable) == 0);
    assert!(core::mem::offset_of!(GpfifoScheduleParams, b_skip_submit) == 1);
    assert!(core::mem::offset_of!(GpfifoScheduleParams, b_skip_enable) == 2);
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The 580 channel-param offsets, asserted as BYTES ON THE WIRE rather than as
    /// `offset_of!` — there is no `#[repr(C)]` mirror to take offsets of, which is the
    /// point of the module docs. Each field is written into a zeroed buffer alone and
    /// its bytes located.
    #[test]
    fn channel_alloc_params_land_at_the_580_offsets() {
        let cases: [(ChannelAllocParams, usize, &[u8]); 9] = [
            (
                ChannelAllocParams {
                    h_object_error: 0x1111_1111,
                    ..Default::default()
                },
                0,
                &0x1111_1111u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    gp_fifo_offset: 0x2222_2222_2222_2222,
                    ..Default::default()
                },
                8,
                &0x2222_2222_2222_2222u64.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    gp_fifo_entries: 0x3333_3333,
                    ..Default::default()
                },
                16,
                &0x3333_3333u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    flags: 0x4444_4444,
                    ..Default::default()
                },
                20,
                &0x4444_4444u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    h_context_share: 0x5555_5555,
                    ..Default::default()
                },
                24,
                &0x5555_5555u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    h_va_space: 0x6666_6666,
                    ..Default::default()
                },
                28,
                &0x6666_6666u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    h_userd_memory_0: 0x7777_7777,
                    ..Default::default()
                },
                32,
                &0x7777_7777u32.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    userd_offset_0: 0x8888_8888_8888_8888,
                    ..Default::default()
                },
                64,
                &0x8888_8888_8888_8888u64.to_le_bytes(),
            ),
            (
                ChannelAllocParams {
                    engine_type: 0x9999_9999,
                    ..Default::default()
                },
                128,
                &0x9999_9999u32.to_le_bytes(),
            ),
        ];
        for (params, offset, want) in cases {
            let mut buf = [0u8; ChannelAllocParams::SIZE];
            params.encode_into(&mut buf).expect("encode");
            assert_eq!(&buf[offset..offset + want.len()], want, "at +{offset}");
            // And nothing else moved: every byte outside the field is still zero.
            let nonzero: Vec<usize> = (0..ChannelAllocParams::SIZE)
                .filter(|&i| buf[i] != 0)
                .collect();
            let expected: Vec<usize> = (offset..offset + want.len()).collect();
            assert_eq!(nonzero, expected, "field at +{offset} spilled");
        }
    }

    /// ★★ The offset that is the whole reason this module is hand-written. At
    /// `ogkm-610` `engineType` is at +132; at `ogkm-580` it is at +128.
    ///
    /// ★★★ **This test asserts the ENCODER, and hardware does not corroborate it.** The
    /// bite was run — the encoder was changed to +132 on a real 580.159.04 driver and the
    /// engine-routing sweep produced byte-identical output. For a channel inside a TSG the
    /// field is inert; the group's `engineType` is what routes. See
    /// [`ChannelAllocParams::engine_type`] for the full measurement. The test stays
    /// because the header says +128 and an encoder that drifts off its header is still a
    /// defect — but it is a claim about the header, not about the GPU, and calling it
    /// hardware evidence would be exactly the green-suite-that-means-nothing this project
    /// has measured before.
    #[test]
    fn engine_type_is_at_128_which_is_the_580_offset_and_not_the_610_one() {
        let mut buf = [0u8; ChannelAllocParams::SIZE];
        ChannelAllocParams {
            engine_type: ENGINE_TYPE_COPY0,
            ..Default::default()
        }
        .encode_into(&mut buf)
        .expect("encode");
        assert_eq!(u32::from_le_bytes(buf[128..132].try_into().unwrap()), 9);
        assert_eq!(
            u32::from_le_bytes(buf[132..136].try_into().unwrap()),
            0,
            "+132 is 610's engineType offset and must be untouched"
        );
    }

    /// A short buffer is a refusal, never a partial encode.
    #[test]
    fn a_short_buffer_is_truncated_and_not_a_partial_encode() {
        let mut buf = [0u8; ChannelAllocParams::SIZE - 1];
        let err = ChannelAllocParams {
            engine_type: 1,
            ..Default::default()
        }
        .encode_into(&mut buf)
        .expect_err("must refuse");
        assert_eq!(
            err,
            AbiError::Truncated {
                c_name: ChannelAllocParams::C_NAME,
                need: ChannelAllocParams::SIZE,
                got: ChannelAllocParams::SIZE - 1,
            }
        );
    }

    /// The engine-type table, by value. `COPY(10)` is `None` and NOT `COPY0 + 10`.
    #[test]
    fn engine_type_copy_refuses_past_the_macro_s_first_arm() {
        assert_eq!(engine_type_copy(0), Some(9));
        assert_eq!(engine_type_copy(1), Some(10));
        assert_eq!(engine_type_copy(9), Some(18));
        assert_eq!(engine_type_copy(10), None);
        assert_eq!(engine_type_copy(u32::MAX), None);
        assert_eq!(ENGINE_TYPE_GRAPHICS, 1);
    }

    /// The GPFIFO entry encoding, by field.
    #[test]
    fn gp_entry_places_address_and_length_in_their_fields() {
        // A 40-bit address and a 64-byte (16-dword) pushbuffer.
        let e = gp_entry(0x0000_00AB_CDEF_0000, 64).expect("encodable");
        assert_eq!(e & 0xFFFF_FFFF, 0xCDEF_0000, "GP_ENTRY0_GET = address 31:2");
        let hi = (e >> 32) as u32;
        assert_eq!(hi & 0xFF, 0xAB, "GP_ENTRY1_GET_HI = address 39:32");
        assert_eq!((hi >> 10) & 0x1F_FFFF, 16, "GP_ENTRY1_LENGTH = dwords");
        assert_eq!(hi >> 31, 0, "GP_ENTRY1_SYNC = PROCEED");
        assert_eq!((hi >> 9) & 1, 0, "GP_ENTRY1_LEVEL = MAIN");
    }

    /// ★ Every bound is a refusal, not a truncation. Each row is a value that WOULD
    /// have produced a wrong-but-runnable entry.
    #[test]
    fn gp_entry_refuses_every_value_it_cannot_represent() {
        // Misaligned address: bit 0 is FETCH_CONDITIONAL, bit 1 is reserved.
        assert_eq!(gp_entry(0x1002, 4), None);
        assert_eq!(gp_entry(0x1001, 4), None);
        // Above 2^40 — GET_HI is eight bits.
        assert_eq!(gp_entry(1 << 40, 4), None);
        // Zero length, and a length that is not a whole dword.
        assert_eq!(gp_entry(0x1000, 0), None);
        assert_eq!(gp_entry(0x1000, 6), None);
        // Past 2^21 dwords — LENGTH is 21 bits.
        assert_eq!(gp_entry(0x1000, (1 << 21) * 4), None);
        // The largest representable one is accepted, so the bound is exact.
        assert!(gp_entry(0x1000, ((1 << 21) - 1) * 4).is_some());
        assert!(gp_entry((1 << 40) - 4, 4).is_some());
    }

    /// ★★ The method header's address field is DWORD-indexed. This is the assertion
    /// that would catch passing a byte offset unshifted, which names a different
    /// register four times further along and does not fault.
    #[test]
    fn method_header_shifts_the_byte_offset_into_a_dword_index() {
        let h = method_header_inc(4, ce::OFFSET_IN_UPPER, 4).expect("encodable");
        assert_eq!(h & 0xFFF, 0x100, "0x400 bytes is method index 0x100");
        assert_eq!((h >> 13) & 0x7, 4, "SUBCHANNEL");
        assert_eq!((h >> 16) & 0x1FFF, 4, "COUNT");
        assert_eq!(h >> 29, 1, "OPCODE = INC_METHOD");
    }

    /// Every bound on the header is a refusal too.
    #[test]
    fn method_header_refuses_every_value_it_cannot_represent() {
        // A method offset that is not dword-aligned has no dword index.
        assert_eq!(method_header_inc(0, 0x401, 1), None);
        // Past the 12-bit address field (0x1000 dwords = 0x4000 bytes).
        assert_eq!(method_header_inc(0, 0x4000, 1), None);
        assert!(method_header_inc(0, 0x3FFC, 1).is_some());
        // Subchannel is 3 bits.
        assert_eq!(method_header_inc(8, 0, 1), None);
        assert!(method_header_inc(7, 0, 1).is_some());
        // Count is 13 bits and zero is not a header.
        assert_eq!(method_header_inc(0, 0, 0), None);
        assert_eq!(method_header_inc(0, 0, 1 << 13), None);
        assert!(method_header_inc(0, 0, (1 << 13) - 1).is_some());
    }

    /// The schedule params are THREE bytes. A fourth would be a different `paramsSize`
    /// and therefore a different request.
    #[test]
    fn schedule_params_are_three_bytes_and_encode_by_position() {
        assert_eq!(GpfifoScheduleParams::SIZE, 3);
        let mut buf = [0u8; 3];
        GpfifoScheduleParams {
            b_enable: 1,
            b_skip_submit: 0,
            b_skip_enable: 0,
        }
        .encode_into(&mut buf)
        .expect("encode");
        assert_eq!(buf, [1, 0, 0]);
    }

    /// The USERD cursors, and that they are DISTINCT. The C reads them at 0x88/0x8C
    /// (`C: src/qemu/nvkvm_gpu_emul.c:4199-4202`); this is the same pair derived from
    /// the dword indices rather than from the C's constants.
    #[test]
    fn userd_cursor_offsets_match_the_c_artifact_s_measured_pair() {
        assert_eq!(USERD_GP_GET, 0x88);
        assert_eq!(USERD_GP_PUT, 0x8C);
        assert_ne!(USERD_GP_GET, USERD_GP_PUT);
    }

    /// ⊘ **A round trip, and it is deliberately labelled as proving nothing on its own.**
    /// [`gp_entry`] and [`gp_entry_decode`] are both ours; agreeing with each other is
    /// what two functions written from the same wrong belief also do
    /// (`never_let_a_test_use_the_thing_under_test_as_its_own_observer`, and the shape
    /// that let a planted mutation survive `MockArch::token_for`). What settles the
    /// encoding is `tests/tests/pushbuffer_abi_oracle.rs`, which builds the entry with
    /// NVIDIA's own `DRF_NUM` over NVIDIA's own field definitions. This test is here to
    /// catch a *regression* in one half, and that is all it is here for.
    #[test]
    fn gp_entry_round_trips_through_our_own_encoder_only() {
        let e = gp_entry(0x00A5_0000_1230, 0x40).expect("encodable");
        let d = gp_entry_decode(e).expect("names method words");
        assert_eq!(d.gpu_va, 0x00A5_0000_1230);
        assert_eq!(d.len_bytes, 0x40);
        assert!(!d.subroutine);
        assert!(!d.sync_wait);
    }

    /// ★★★ A **control** entry decodes to nothing. `LENGTH == 0` makes entry1's low byte
    /// `OPCODE`, not `GET_HI`, so reading an address out of it fabricates a pointer.
    #[test]
    fn a_zero_length_gpfifo_entry_is_refused_and_not_read_as_an_address() {
        for opcode in 0u64..=3 {
            // Entry1 = the opcode alone (LENGTH = 0); entry0 = a plausible address.
            let entry = 0x0000_1000u64 | (opcode << 32);
            assert_eq!(
                gp_entry_decode(entry),
                None,
                "opcode {opcode} is a control entry and names no method words"
            );
        }
        // One dword of methods is the smallest entry that DOES name any.
        assert!(gp_entry_decode(0x0000_1000 | (1u64 << (32 + 10))).is_some());
    }

    /// The `LEVEL`/`SYNC` bits are carried rather than dropped, and neither is confused
    /// for the other or for the length.
    #[test]
    fn gp_entry_decode_reports_level_and_sync_without_disturbing_the_length() {
        let base = gp_entry(0x2000, 8).expect("encodable");
        let sub = base | (1u64 << (32 + 9));
        let wait = base | (1u64 << (32 + 31));
        let d = gp_entry_decode(sub).expect("range");
        assert!(d.subroutine && !d.sync_wait && d.len_bytes == 8 && d.gpu_va == 0x2000);
        let d = gp_entry_decode(wait).expect("range");
        assert!(d.sync_wait && !d.subroutine && d.len_bytes == 8 && d.gpu_va == 0x2000);
    }

    /// The header decoder agrees with the header ENCODER on the one form the encoder
    /// writes — again a regression check only, for
    /// [`gp_entry_round_trips_through_our_own_encoder_only`]'s reason.
    #[test]
    fn method_header_decode_reads_back_what_method_header_inc_wrote() {
        let h = method_header_inc(4, ce::OFFSET_IN_UPPER, 4).expect("encodable");
        let d = method_header_decode(h).expect("a defined format");
        assert_eq!(d.form, MethodForm::Incrementing);
        assert_eq!(d.method, ce::OFFSET_IN_UPPER);
        assert_eq!(d.subchannel, 4);
        assert_eq!(d.arg_words, 4);
    }

    /// ★★★ **The refusal.** `RESERVED6` is enumerated by the class header with no
    /// format, and `GRP2_USE_TERT` defines exactly one `TERT_OP`. Both are `None` — a
    /// guessed argument count is a parser walking onto its own data.
    #[test]
    fn the_two_undefined_header_encodings_are_refused_and_not_sized() {
        // RESERVED6, with an otherwise perfectly ordinary-looking body.
        assert_eq!(method_header_decode((6 << 29) | (4 << 16) | 0xC0), None);
        // GRP2 with each TERT_OP the header does not define.
        for tert in 1u32..=3 {
            assert_eq!(
                method_header_decode((2 << 29) | (tert << 16) | 0xC0),
                None,
                "GRP2 TERT_OP {tert} has no enumerated format"
            );
        }
        // …and the one it does define is sized.
        assert!(method_header_decode(2 << 29).is_some());
    }

    /// Every enumerated `SEC_OP` except `RESERVED6` is sizable, quantified over
    /// [`sec_op::ALL`] rather than over a list written here.
    #[test]
    fn every_enumerated_sec_op_but_reserved6_has_a_size() {
        let mut sized = 0usize;
        for op in sec_op::ALL {
            let h = (op << 29) | 0xC0;
            match method_header_decode(h) {
                Some(_) => sized += 1,
                None => assert_eq!(op, sec_op::RESERVED6, "SEC_OP {op} lost its format"),
            }
        }
        assert_eq!(sized, sec_op::ALL.len() - 1);
    }

    /// `NVC56F_DMA_NOP` is `0x00000000`, i.e. the legacy form with a zero count — so a
    /// zero-filled pushbuffer is a run of NOPs and the parser never desynchronises on it.
    #[test]
    fn an_all_zero_word_is_a_nop_that_consumes_no_arguments() {
        let d = method_header_decode(0).expect("NOP is a defined format");
        assert_eq!(d.form, MethodForm::Legacy);
        assert_eq!(d.arg_words, 0);
        assert_eq!(d.method, 0);
    }

    /// The immediate form carries its datum in the header and consumes **no** words. A
    /// decoder that gave it `count` arguments would swallow the next method.
    #[test]
    fn the_immediate_form_consumes_no_argument_words() {
        let d = method_header_decode((4 << 29) | (0x1234 << 16) | 0xC0).expect("defined");
        assert_eq!(d.form, MethodForm::Immediate);
        assert_eq!(d.arg_words, 0);
        assert_eq!(d.immd, 0x1234);
    }

    /// ★★★ **The geography that decides `#128`.** The usermode timer mirror and the
    /// doorbell are in the SAME 4 KiB page (16 bytes apart), so no page-granular mechanism
    /// can pass one through and trap the other; the PTIMER page is a whole page of timer
    /// with no doorbell in it. If this ever stops holding, the read-native design's premise
    /// has moved and the design must be re-read, not patched.
    #[test]
    fn the_usermode_timer_mirror_shares_a_page_with_the_doorbell_but_ptimer_does_not() {
        const PAGE: u64 = 4096;
        // The mirror and the doorbell: same page, and adjacent.
        assert_eq!(
            USERMODE_TIME_0 / PAGE,
            USERMODE_NOTIFY_CHANNEL_PENDING / PAGE
        );
        assert_eq!(
            USERMODE_TIME_1 / PAGE,
            USERMODE_NOTIFY_CHANNEL_PENDING / PAGE
        );
        assert_eq!(USERMODE_NOTIFY_CHANNEL_PENDING - USERMODE_TIME_0, 0x10);
        // The PTIMER page: page-aligned, exactly one page, both words inside it, and the
        // doorbell's offset within the usermode window is NOT a timer register here.
        assert_eq!(PTIMER_BAR0_BASE % PAGE, 0);
        assert_eq!(PTIMER_PAGE_SIZE, PAGE);
        const { assert!(PTIMER_PAGE_TIME_1 + 4 <= PTIMER_PAGE_SIZE) };
        assert_eq!(PTIMER_BAR0_BASE + PTIMER_PAGE_TIME_0, 0x9400);
        assert_eq!(PTIMER_BAR0_BASE + PTIMER_PAGE_TIME_1, 0x9410);
        // `Nv01TimerMap` stops well short of the page it lives in — the mapping RM sizes
        // for an `NV01_TIMER` object is NOT the whole range the mmap whitelist permits.
        const { assert!(NV01_TIMER_MAP_SIZE < PTIMER_PAGE_SIZE) };
        assert_eq!(NV01_TIMER_MAP_SIZE, PTIMER_PAGE_TIME_1 + 4);
    }

    /// The high word carries 29 bits, so bits 61..63 of a composed reading are always
    /// zero however the register is decorated.
    #[test]
    fn compose_masks_the_high_word_to_its_29_significant_bits() {
        assert_eq!(
            ptimer_compose(0xffff_ffff, 0xffff_ffff),
            0x1fff_ffff_ffff_ffff
        );
        assert_eq!(ptimer_compose(0xe000_0000, 0), 0);
        assert_eq!(ptimer_compose(1, 2), (1 << 32) | 2);
    }

    /// A stable counter is read in one round, with exactly three register reads.
    #[test]
    fn a_stable_counter_is_sampled_in_one_round() {
        let mut reads = 0usize;
        let v = ptimer_sample::<()>(PTIMER_PAGE_TIME_1, PTIMER_PAGE_TIME_0, |off| {
            reads += 1;
            Ok(if off == PTIMER_PAGE_TIME_1 { 7 } else { 42 })
        })
        .expect("stable");
        assert_eq!(v, ptimer_compose(7, 42));
        assert_eq!(reads, 3);
    }

    /// ★ A carry between the two words is RETRIED, not returned. Round 1 straddles (the
    /// second `hi` differs); round 2 is clean. Returning round 1's `(hi, lo)` would have
    /// been wrong by ~4.29 s and perfectly plausible.
    #[test]
    fn a_carry_between_the_words_is_retried_rather_than_returned() {
        // hi, lo, hi' | hi, lo, hi'
        let mut script = [5u32, 0xffff_ffff, 6, 6, 0x0000_0001, 6].into_iter();
        let v = ptimer_sample::<()>(PTIMER_PAGE_TIME_1, PTIMER_PAGE_TIME_0, |_| {
            Ok(script.next().expect("script"))
        })
        .expect("second round is coherent");
        assert_eq!(v, ptimer_compose(6, 1));
    }

    /// ⊘ A counter that carries on every single round REFUSES. It does not fall back to
    /// the last pair, and it does not return zero — the two answers that read as a working
    /// timer. `PTIMER_SAMPLE_ROUNDS` rounds of three reads is the whole budget.
    #[test]
    fn a_counter_that_never_settles_refuses_rather_than_guessing() {
        let mut reads = 0usize;
        let e = ptimer_sample::<()>(PTIMER_PAGE_TIME_1, PTIMER_PAGE_TIME_0, |off| {
            reads += 1;
            // Every `hi` read differs from the previous one.
            Ok(if off == PTIMER_PAGE_TIME_1 {
                reads as u32
            } else {
                0
            })
        })
        .expect_err("must not invent a reading");
        assert_eq!(e, PtimerSampleError::Incoherent);
        assert_eq!(reads, PTIMER_SAMPLE_ROUNDS * 3);
    }

    /// A transport refusal is carried out, not swallowed into a zero.
    #[test]
    fn a_failed_register_read_propagates_instead_of_reading_as_zero() {
        let e = ptimer_sample::<&str>(PTIMER_PAGE_TIME_1, PTIMER_PAGE_TIME_0, |_| {
            Err("the mapping was refused")
        })
        .expect_err("must not answer");
        assert_eq!(e, PtimerSampleError::Read("the mapping was refused"));
    }

    /// USERD is 512 bytes and both cursors are inside it — a model answering less would
    /// size a mapping that stops short of the produce cursor.
    #[test]
    fn userd_is_large_enough_for_both_cursors() {
        assert_eq!(USERD_SIZE, 512);
        const { assert!(USERD_GP_PUT + 4 <= USERD_SIZE) };
        const { assert!(USERD_GP_GET + 4 <= USERD_SIZE) };
    }

    /// The `NVOS33` round trip, including a negative descriptor.
    #[test]
    fn nvos33_round_trips_and_keeps_a_negative_descriptor_negative() {
        let p = Nvos33ParametersWithFd {
            h_client: 0xC1D0_0001,
            h_device: 0xCAFE_0001,
            h_memory: 0xCAFE_0007,
            offset: 0,
            length: 0x1000,
            p_linear_address: 0,
            status: 0,
            flags: 0,
            fd: -1,
        };
        let mut buf = [0u8; Nvos33ParametersWithFd::SIZE];
        p.encode_into(&mut buf).expect("encode");
        assert_eq!(Nvos33ParametersWithFd::decode(&buf).expect("decode"), p);
        assert_eq!(Nvos33ParametersWithFd::default().fd, -1);
    }

    /// The doorbell offset and the window it lives in — a store past the window is a
    /// store into another object's registers.
    #[test]
    fn the_doorbell_offset_is_inside_the_usermode_window() {
        assert_eq!(USERMODE_NOTIFY_CHANNEL_PENDING, 0x90);
        const { assert!(USERMODE_NOTIFY_CHANNEL_PENDING + 4 <= USERMODE_WINDOW_SIZE) };
        assert_eq!(MMAP_FILE_OFFSET, 0);
    }

    /// The memory-allocation params' offsets, field by field, with the same
    /// nothing-else-moved check as the channel params.
    #[test]
    fn memory_allocation_params_land_at_the_580_offsets() {
        let cases: [(NvMemoryAllocationParams, usize, &[u8]); 5] = [
            (
                NvMemoryAllocationParams {
                    owner: 0x1111_1111,
                    ..Default::default()
                },
                0,
                &0x1111_1111u32.to_le_bytes(),
            ),
            (
                NvMemoryAllocationParams {
                    kind: 0x2222_2222,
                    ..Default::default()
                },
                4,
                &0x2222_2222u32.to_le_bytes(),
            ),
            (
                NvMemoryAllocationParams {
                    attr: 0x3333_3333,
                    ..Default::default()
                },
                24,
                &0x3333_3333u32.to_le_bytes(),
            ),
            (
                NvMemoryAllocationParams {
                    size: 0x4444_4444_4444_4444,
                    ..Default::default()
                },
                64,
                &0x4444_4444_4444_4444u64.to_le_bytes(),
            ),
            (
                NvMemoryAllocationParams {
                    alignment: 0x5555_5555_5555_5555,
                    ..Default::default()
                },
                72,
                &0x5555_5555_5555_5555u64.to_le_bytes(),
            ),
        ];
        for (params, offset, want) in cases {
            let mut buf = [0u8; NvMemoryAllocationParams::SIZE];
            params.encode_into(&mut buf).expect("encode");
            assert_eq!(&buf[offset..offset + want.len()], want, "at +{offset}");
            let nonzero: Vec<usize> = (0..NvMemoryAllocationParams::SIZE)
                .filter(|&i| buf[i] != 0)
                .collect();
            let expected: Vec<usize> = (offset..offset + want.len()).collect();
            assert_eq!(nonzero, expected, "field at +{offset} spilled");
        }
    }

    /// ★ `ATTR_CONTIGUOUS_VIDMEM` is a FIELD VALUE, not a bit mask — the mistake the
    /// `NVOS02` flags record having made against real hardware. `PHYSICALITY` is `28:27`
    /// and `CONTIGUOUS` is 2; `LOCATION` is `26:25` and `VIDMEM` is 0.
    ///
    /// ★★ **This test is narrower than its first draft, and the narrowing is the
    /// finding.** It also asserted `!= 1 << 28`, and that assertion FIRED: `2 << 27` and
    /// `1 << 28` are the same number. The mask reading and the field reading happen to
    /// agree here, so no test can distinguish them by value — only the *decode* below can
    /// say the constant means "field 28:27 holds 2", and `!= 1 << 27` is the one mask
    /// spelling that is genuinely a different number. Asserting the rest would have been
    /// a false claim that happened to be checkable.
    #[test]
    fn the_vidmem_attribute_is_a_field_value_and_not_a_bit_mask() {
        assert_eq!(ATTR_CONTIGUOUS_VIDMEM, 0x1000_0000);
        assert_eq!(
            (ATTR_CONTIGUOUS_VIDMEM >> 27) & 0b11,
            2,
            "PHYSICALITY 28:27"
        );
        assert_eq!(
            (ATTR_CONTIGUOUS_VIDMEM >> 25) & 0b11,
            0,
            "LOCATION 26:25 = VIDMEM"
        );
        assert_ne!(ATTR_CONTIGUOUS_VIDMEM, 1 << 27);
    }

    /// The same up-front length check on the memory params, whose last written field is
    /// at +72 of 128 — an even wider gap than the channel params'.
    #[test]
    fn memory_params_refuse_a_buffer_that_would_encode_a_shorter_request() {
        let mut buf = [0u8; NvMemoryAllocationParams::SIZE - 1];
        let err = NvMemoryAllocationParams {
            size: 0x1000,
            ..Default::default()
        }
        .encode_into(&mut buf)
        .expect_err("must refuse");
        assert_eq!(
            err,
            AbiError::Truncated {
                c_name: NvMemoryAllocationParams::C_NAME,
                need: NvMemoryAllocationParams::SIZE,
                got: NvMemoryAllocationParams::SIZE - 1,
            }
        );
    }

    /// ★★ The three TSG-side control numbers are DISTINCT and none is the channel-side
    /// schedule. Confusing `0xa06c0101` (schedule a group) with `0xa06c0102` (bind a
    /// group to an engine) reorders the sequence the token control depends on.
    #[test]
    fn the_tsg_controls_are_three_distinct_commands() {
        assert_eq!(NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, 0xa06c_0101);
        assert_eq!(NVA06C_CTRL_CMD_BIND, 0xa06c_0102);
        assert_eq!(NVA06F_CTRL_CMD_GPFIFO_SCHEDULE, 0xa06f_0103);
        assert_ne!(NVA06C_CTRL_CMD_GPFIFO_SCHEDULE, NVA06C_CTRL_CMD_BIND);
        assert_ne!(
            NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
            NVA06F_CTRL_CMD_GPFIFO_SCHEDULE
        );
        assert_eq!(BIND_PARAMS_SIZE, 4);
        assert_eq!(WORK_SUBMIT_TOKEN_PARAMS_SIZE, 4);
    }

    /// ★★★ **E9.** The channel-side bind is `0xa06f0104` and is a *fourth* command, not a
    /// spelling of any of the three above.
    ///
    /// ⊘ `0xa06c0102` binds a channel **group**; `0xa06f0104` binds a bare **channel**.
    /// They differ in one nibble of the class id and in which object the guest issues them
    /// on, and the guest's scrubber channel — the one that reaches us — has no group.
    #[test]
    fn the_channel_side_bind_is_a_fourth_distinct_command() {
        assert_eq!(NVA06F_CTRL_CMD_BIND, 0xa06f_0104);
        for other in [
            NVA06C_CTRL_CMD_GPFIFO_SCHEDULE,
            NVA06C_CTRL_CMD_BIND,
            NVA06F_CTRL_CMD_GPFIFO_SCHEDULE,
        ] {
            assert_ne!(
                NVA06F_CTRL_CMD_BIND, other,
                "the channel-side bind collided with another FIFO control"
            );
        }
        assert_eq!(BindParams::SIZE, 4);
    }

    /// ★★★ `[measured]` **The real GA106's own request, byte for byte.**
    ///
    /// `traces/real_ga106/rpc_transcript_real_ga106.txt:63` — `cmd=0xa06f0104 psize=4
    /// head=0b 00 00 00`. Decoding those four bytes must yield `NV2080_ENGINE_TYPE_COPY2`,
    /// and the reply body must be the same four bytes back.
    ///
    /// ⚠ This is the test that would have caught the C's empty row: decoding `dlen = 0`
    /// gives `engine_type = 0` = `NV2080_ENGINE_TYPE_NULL`, which is not what the hardware
    /// was asked and not what it answered.
    #[test]
    fn the_measured_ga106_bind_request_decodes_to_copy2_and_echoes_back() {
        let wire = [0x0b, 0x00, 0x00, 0x00];
        let got = decode_bind(&wire).expect("four bytes is the whole struct");
        assert_eq!(
            got.engine_type,
            engine_type_copy(2).expect("COPY2 is within the first ten"),
            "a real GA106 binds its scrubber channel to COPY2 (11), and 11 must decode to \
             exactly that — if this reads as COPY0 the ordinal base is wrong"
        );
        assert_eq!(got.engine_type, 11, "COPY0 is 9, so COPY2 is 11");
        assert_ne!(
            got.engine_type, 0,
            "⊘ zero is NV2080_ENGINE_TYPE_NULL and is what the C's EMPTY captured row \
             decodes to — the value this test exists to distinguish from a measurement"
        );
        assert_eq!(&encode_bind(&got)[..], &wire[..]);
    }

    /// A short params image is a decode failure, not a zero.
    ///
    /// ⊘ The tempting alternative — pad and read what is there — would turn a truncated
    /// request into a bind to whatever engine the low bytes spell.
    #[test]
    fn a_bind_params_image_shorter_than_its_struct_is_refused() {
        for n in 0..BindParams::SIZE {
            let err = decode_bind(&vec![0xffu8; n]).expect_err("must refuse");
            assert_eq!(err, GpfifoScheduleError::ShortParams { got: n });
        }
    }

    /// ★★ The refusal statuses are answers the **code** produces, and neither is
    /// `NV_ERR_NOT_SUPPORTED`.
    ///
    /// ⊘ `0x56` is the FSM's *"nobody claimed this"* signature. A refusal that reused it
    /// would be indistinguishable, in the guest's own dmesg, from this port having no code
    /// for the command at all — which is the confusion that cost the schedule rung weeks.
    #[test]
    fn the_bind_refusals_are_answers_the_code_produces_and_never_the_unclaimed_signature() {
        for s in [BIND_UNKNOWN_ENGINE_STATUS, BIND_REFUSED_STATUS] {
            assert!(
                BIND_STATUSES_THE_CODE_PRODUCES.contains(&s),
                "{s:#x} is not a status the bind path produces, so answering it would be \
                 describing a check RM does not perform"
            );
            assert_ne!(s, 0x56, "NV_ERR_NOT_SUPPORTED is the unclaimed signature");
            assert_ne!(s, 0, "a refusal that answers NV_OK is not a refusal");
        }
        assert_ne!(
            BIND_UNKNOWN_ENGINE_STATUS, BIND_REFUSED_STATUS,
            "★ the two refusals say different things — an engine this device does not have \
             versus a channel that cannot be routed — and collapsing them throws away the \
             only diagnosis the guest gets"
        );
    }

    /// ★★★ **The header is INCOMPLETE, and this test is the record of it.**
    ///
    /// `ctrla06fgpfifo.h:91-95` lists three statuses. The bind path produces a fourth —
    /// `NV_ERR_OBJECT_NOT_FOUND` (`0x57`) from `kernel_fifo_gm107.c:736` — for the case this
    /// port most needs to answer. A first cut made the documented list the acceptance gate
    /// for refusals, which would have **rejected the true status and admitted only wrong
    /// ones**.
    ///
    /// ⊘ The lesson is not "distrust headers". It is that a citation proves a claim is
    /// *sourced*, never that the source is *complete* — the same shape as the C oracle's
    /// empty rows, where a real row corroborated a false number.
    #[test]
    fn the_documented_status_list_is_a_strict_subset_of_what_the_code_produces() {
        for d in BIND_DOCUMENTED_STATUSES {
            assert!(
                BIND_STATUSES_THE_CODE_PRODUCES.contains(d),
                "{d:#x} is documented but no path produces it — that would make the header \
                 wrong in the OTHER direction, which is a different finding and wants its own \
                 citation"
            );
        }
        assert!(
            BIND_STATUSES_THE_CODE_PRODUCES.len() > BIND_DOCUMENTED_STATUSES.len(),
            "★ if these are equal the header caught up with the code (or someone trimmed the \
             code list to match the header, which is the failure this test exists to catch). \
             Re-read kernel_fifo_gm107.c:736 before changing this"
        );
        assert!(
            !BIND_DOCUMENTED_STATUSES.contains(&BIND_UNKNOWN_ENGINE_STATUS),
            "the whole point: the status we answer for an absent engine is NOT in the \
             header's list"
        );
    }

    /// ★★★ **The collision.** Raw `0x13` is `NVDEC0` in `NV2080` space and `COPY10` in `RM`
    /// space, so a bind naming a video decoder must never convert to a copy engine.
    ///
    /// ⊘ This is the test a raw integer comparison passes trivially and wrongly. The
    /// identity ranges below it are what make such a comparison *look* right.
    #[test]
    fn the_two_engine_type_spaces_collide_and_the_conversion_does_not() {
        // The ranges where the two spaces genuinely agree — and the reason a raw compare
        // survives every obvious test.
        assert_eq!(nv2080_to_rm_engine_type(ENGINE_TYPE_GRAPHICS), Some(1));
        assert_eq!(nv2080_to_rm_engine_type(ENGINE_TYPE_COPY0), Some(9));
        assert_eq!(
            nv2080_to_rm_engine_type(11),
            Some(11),
            "COPY2, the measured one"
        );
        assert_eq!(
            nv2080_to_rm_engine_type(0x12),
            Some(0x12),
            "COPY9, the last agreeing"
        );

        // ★ And the first ordinal past them, where they do not.
        assert_eq!(
            nv2080_to_rm_engine_type(0x13),
            None,
            "0x13 is NVDEC0 in the space this parameter is written in; RM_ENGINE_TYPE_COPY10 \
             is also 0x13, so a raw compare against an RM-space table would bind a video \
             decoder to the eleventh copy engine"
        );
        assert_eq!(
            nv2080_to_rm_engine_type(0x34),
            None,
            "0x34 IS NV2080's COPY10 — refused because this port advertises no such row, not \
             because the ordinal is meaningless"
        );

        // The one modelled row where the spaces disagree, converted rather than passed through.
        assert_eq!(
            nv2080_to_rm_engine_type(NV2080_ENGINE_TYPE_SW),
            Some(RM_ENGINE_TYPE_SW)
        );
        assert_ne!(
            NV2080_ENGINE_TYPE_SW, RM_ENGINE_TYPE_SW,
            "★ 0x22 vs 0x2d — the cheapest proof these are different enums, and the reason \
             ga10x.rs's SOFTWARE row carrying 0x2d confirms our tables are in RM space"
        );

        // Nothing bindable at either end of the space.
        for dead in [0u32, 0x54, 0x2d, 0xffff, 0xffff_ffff] {
            assert_eq!(
                nv2080_to_rm_engine_type(dead),
                None,
                "{dead:#x} names no engine this device advertises"
            );
        }
    }

    /// The five host-FIFO semaphore methods are consecutive dwords starting at `0x5c`,
    /// so ONE incrementing header of count 5 covers them. If they were not consecutive
    /// the pushbuffer in the rung above would silently address the wrong registers.
    #[test]
    fn the_fifo_semaphore_methods_are_five_consecutive_dwords() {
        let m = [
            fifo::SEM_ADDR_LO,
            fifo::SEM_ADDR_HI,
            fifo::SEM_PAYLOAD_LO,
            fifo::SEM_PAYLOAD_HI,
            fifo::SEM_EXECUTE,
        ];
        assert_eq!(m[0], 0x5C);
        for (i, w) in m.iter().enumerate() {
            assert_eq!(*w, 0x5C + 4 * i as u32, "method {i}");
        }
        assert_eq!(fifo::SEM_EXECUTE_RELEASE_32BIT, 1);
        // The C writes exactly this header: INC, count 5, subchannel 0, method 0x5c.
        // (`C: src/qemu/nvkvm_gpu_emul.c:8598`.)
        let h = method_header_inc(0, fifo::SEM_ADDR_LO, 5).expect("encodable");
        assert_eq!(h, (1 << 29) | (5 << 16) | (0x5C >> 2));
    }

    /// ★ The copy engine's four address methods are consecutive too, so the copy's
    /// operands go out under one header of count 4. Same failure shape as the row above:
    /// a gap would make the header address `PITCH_IN`/`PITCH_OUT` with an address.
    #[test]
    fn the_copy_engine_address_methods_are_four_consecutive_dwords() {
        let m = [
            ce::OFFSET_IN_UPPER,
            ce::OFFSET_IN_LOWER,
            ce::OFFSET_OUT_UPPER,
            ce::OFFSET_OUT_LOWER,
        ];
        for (i, w) in m.iter().enumerate() {
            assert_eq!(*w, 0x400 + 4 * i as u32, "method {i}");
        }
        // …and LINE_LENGTH_IN/LINE_COUNT are a consecutive PAIR, one header of count 2.
        assert_eq!(ce::LINE_COUNT, ce::LINE_LENGTH_IN + 4);
        // …and the three semaphore methods are a consecutive RUN of three.
        assert_eq!(ce::SET_SEMAPHORE_B, ce::SET_SEMAPHORE_A + 4);
        assert_eq!(ce::SET_SEMAPHORE_PAYLOAD, ce::SET_SEMAPHORE_A + 8);
    }

    /// ★★ The eight bytes that decide the runlist: version 1 and the SAME engine ordinal
    /// the group was allocated with. A zeroed image is `engineType = 0`, which is the
    /// C's proven bug — so `Default` must not be mistaken for a usable value.
    #[test]
    fn the_ce_alloc_params_carry_the_engine_ordinal_at_plus_four() {
        let mut b = [0xEEu8; CeAllocParams::SIZE];
        CeAllocParams {
            version: CeAllocParams::VERSION_1,
            engine_type: ENGINE_TYPE_COPY0,
        }
        .encode_into(&mut b)
        .expect("encodable");
        assert_eq!(b, [1, 0, 0, 0, 9, 0, 0, 0]);
        // The defaulted struct is the BUG, and it is spelled out so that a future caller
        // reaching for `..Default::default()` sees why it is refused at review.
        let mut z = [0xEEu8; CeAllocParams::SIZE];
        CeAllocParams::default().encode_into(&mut z).expect("enc");
        assert_eq!(z, [0u8; 8], "a defaulted CE alloc IS engineType 0");
    }

    /// A short buffer is refused, never truncated — the same rule as every other encoder
    /// in this module.
    #[test]
    fn a_short_ce_alloc_image_is_refused() {
        let mut b = [0u8; CeAllocParams::SIZE - 1];
        assert!(
            CeAllocParams {
                version: 1,
                engine_type: 9
            }
            .encode_into(&mut b)
            .is_err()
        );
    }
}
