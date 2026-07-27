//! The driver-version table: nvproxy's inherit-then-mutate model, and the
//! **exact** version boundaries it turns on.
//!
//! # Why exact boundaries and not majors
//!
//! The C artifact keys its ABI profile on the **major version alone**
//! (`nvidia-gpu-passthrough/src/common/nvkvm_abi.h:112-121`,
//! `nvkvm_abi_id_for_major`). That is too coarse and demonstrably so: the
//! `NVOS46_PARAMETERS` growth lands at **580.65.06**, not at 580.0
//! (`gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1059` switches to
//! `NVOS46_PARAMETERS_V580` at exactly that entry), and `NVOS47_PARAMETERS`
//! grew at **550.54.04** (`gvisor/pkg/abi/nvgpu/frontend.go:707-710`), mid-major
//! in both cases. A major-only key cannot express either, so it is right by
//! luck for the releases that happen to exist and wrong for the ones that do
//! not.
//!
//! It is also wrong in a *second* way: `nvkvm_abi_by_id` returns the 570 profile
//! for any unrecognised id (`nvkvm_abi.h:105-110`), so an unknown driver
//! silently gets 575's struct sizes. Here an unknown-and-too-old version is
//! [`AbiError::NoTableForVersion`]. MISS = FAULT, never a nearest-neighbour
//! guess.
//!
//! # The supported range, and why it starts where it does
//!
//! The oldest table is **550.54.04**. Below that, `NVOS47_PARAMETERS` is the
//! 40-byte pre-`size` shape, which this crate does not carry
//! (`crate::transcribed`'s module doc says why: no supported driver is that old,
//! and an untested transcription is worse than an absent one). So versions below
//! 550.54.04 are refused rather than decoded with a layout that is wrong by 8
//! bytes in the middle of the struct.

use crate::generated::{classes, ctrl, nvos, rpc};
use crate::transcribed::Nvos46ParametersPre580;
use crate::view::{
    AllocReq, AllocWire, ClientAllocFacts, ControlReq, DeviceAllocFacts, DupReq, FreeReq,
    MapMemoryDma, PdbAperture, RpcEnvelope, SetPageDir, UnmapMemoryDma, rpc_payload_len,
};
use crate::wire::{AbiError, u32_at};
use crate::{DriverAbi, DriverVersion};
use kayfabe_arch::ids::ClassId;

/// Which `NVOS46_PARAMETERS` shape a driver version uses.
///
/// The only versioned layout in this milestone's slice — recorded as an explicit
/// enum rather than a size, because the *offsets* move too and a size alone
/// would let a caller compute `status` at the wrong place (which is `#81`, the
/// C artifact's own `nvos46_status_off` table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapDmaWire {
    /// 56 bytes; `dmaOffset` @ +40, `status` @ +48.
    Pre580_65_06,
    /// 64 bytes; `flags2` @ +36, `kindOverride` @ +40, `dmaOffset` @ +48,
    /// `status` @ +56.
    From580_65_06,
}

/// One driver version's ABI table.
///
/// nvproxy's `driverABI` is four handler maps
/// (`version.go:100-107`); this is the same idea at the size this slice needs —
/// one entry per versioned layout, and nothing for the layouts that do not vary.
/// Adding a versioned struct means adding a field here and a line to each table,
/// which is the ~14-51-line delta `mode2_abi_agnostic_layer.md` §2.1 measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverAbiTable {
    version: DriverVersion,
    map_dma: MapDmaWire,
    /// Why this entry exists — kept in the data so a reader of the table sees
    /// the boundary's justification without leaving the file.
    pub note: &'static str,
}

/// The registry, in **ascending** version order.
///
/// `table_for` picks the newest entry `<= requested`, which is inherit-then-
/// mutate expressed as data: an entry only exists where something changed.
pub const TABLES: &[DriverAbiTable] = &[
    DriverAbiTable {
        version: DriverVersion {
            major: 550,
            minor: 54,
            patch: 4,
        },
        map_dma: MapDmaWire::Pre580_65_06,
        note: "oldest supported: NVOS47 gained `size` here \
               (gvisor/pkg/abi/nvgpu/frontend.go:707-710, NVOS47_PARAMETERS_V550)",
    },
    DriverAbiTable {
        version: DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        },
        map_dma: MapDmaWire::From580_65_06,
        note: "NVOS46 gained flags2+kindOverride \
               (gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1059)",
    },
    DriverAbiTable {
        version: DriverVersion {
            major: 610,
            minor: 43,
            patch: 2,
        },
        map_dma: MapDmaWire::From580_65_06,
        note: "the vendored ogkm tag every generated layout in this crate came \
               from; carries no delta, and exists so the generation source is \
               visible in the table it produced",
    },
];

/// The driver version this project's bench actually runs
/// (`docs/reference/rm_semantics_measured.md` §0).
///
/// It sits **above** 580.65.06, so the bench is on the 64-byte `NVOS46` — which
/// is what the C artifact's *runtime* profile also selects
/// (`nvkvm_abi.h:79-87`), and what its *parity test* does not
/// (`abi_parity_test.go:68` asserts 56 unconditionally). The two disagree; the
/// runtime is right.
pub const BENCH_DRIVER: DriverVersion = DriverVersion {
    major: 580,
    minor: 159,
    patch: 4,
};

/// Select the ABI table for a driver version: the newest entry `<= version`.
///
/// # Errors
///
/// [`AbiError::NoTableForVersion`] if `version` predates every entry. There is no
/// fallback entry by design — see the module doc.
pub fn table_for(version: DriverVersion) -> Result<&'static DriverAbiTable, AbiError> {
    TABLES
        .iter()
        .rev()
        .find(|t| t.version <= version)
        .ok_or(AbiError::NoTableForVersion {
            major: version.major,
            minor: version.minor,
            patch: version.patch,
        })
}

impl DriverAbiTable {
    /// Which `NVOS46_PARAMETERS` shape this version speaks.
    #[must_use]
    pub fn map_dma_wire(&self) -> MapDmaWire {
        self.map_dma
    }

    /// `sizeof(NVOS46_PARAMETERS)` for this version — the value an ioctl's size
    /// field must match.
    #[must_use]
    pub fn map_dma_size(&self) -> usize {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => Nvos46ParametersPre580::SIZE,
            MapDmaWire::From580_65_06 => nvos::Nvos46Parameters::SIZE,
        }
    }

    /// Offset of `NVOS46_PARAMETERS::status` for this version.
    ///
    /// The C artifact carries the same two numbers by hand
    /// (`nvkvm_abi.h:66,76,86`: 48, 48, 56) because writing the status to the
    /// wrong offset was bug `#81`.
    #[must_use]
    pub fn map_dma_status_offset(&self) -> usize {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => 48,
            MapDmaWire::From580_65_06 => 56,
        }
    }

    /// Decode `NV_ESC_RM_FREE` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_free(&self, bytes: &[u8]) -> Result<FreeReq, AbiError> {
        let p = nvos::Nvos00Parameters::decode(bytes)?;
        Ok(FreeReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_old,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` parameters in the v1 (`NVOS21`) shape.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_alloc_v1(&self, bytes: &[u8]) -> Result<AllocReq, AbiError> {
        let p = nvos::Nvos21Parameters::decode(bytes)?;
        Ok(AllocReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_new,
            class: p.h_class,
            params_ptr: p.p_alloc_parms,
            // Declared absent, exactly as nvproxy declares it
            // (`frontend.go:322-324` returns 0 rather than panicking).
            rights_requested: 0,
            params_size: p.params_size,
            wire: AllocWire::V1,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` parameters in the v2 (`NVOS64`) shape.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_alloc_v2(&self, bytes: &[u8]) -> Result<AllocReq, AbiError> {
        let p = nvos::Nvos64Parameters::decode(bytes)?;
        Ok(AllocReq {
            client: p.h_root,
            parent: p.h_object_parent,
            handle: p.h_object_new,
            class: p.h_class,
            params_ptr: p.p_alloc_parms,
            rights_requested: p.p_rights_requested,
            params_size: p.params_size,
            wire: AllocWire::V2,
        })
    }

    /// Decode `NV_ESC_RM_ALLOC` by the size the **ioctl** declared, which is how
    /// RM itself and nvproxy discriminate the two shapes
    /// (`gvisor/pkg/abi/nvgpu/frontend.go:290-295`, `GetRmAllocParamObj(isNVOS64)`).
    ///
    /// The discriminator is the ioctl's own size word, **not** `bytes.len()`:
    /// the buffer may legitimately be longer, and choosing the shape from a
    /// length the guest controls indirectly is how you get a 32-byte struct
    /// parsed as a 48-byte one.
    ///
    /// # Errors
    ///
    /// [`AbiError::UnknownAllocWire`] if `ioctl_size` is neither shape's size;
    /// [`AbiError::Truncated`] if the buffer is short.
    pub fn decode_alloc(&self, bytes: &[u8], ioctl_size: usize) -> Result<AllocReq, AbiError> {
        if ioctl_size == nvos::Nvos21Parameters::SIZE {
            self.decode_alloc_v1(bytes)
        } else if ioctl_size == nvos::Nvos64Parameters::SIZE {
            self.decode_alloc_v2(bytes)
        } else {
            Err(AbiError::UnknownAllocWire { ioctl_size })
        }
    }

    /// Decode `NV_ESC_RM_CONTROL` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_control(&self, bytes: &[u8]) -> Result<ControlReq, AbiError> {
        let p = nvos::Nvos54Parameters::decode(bytes)?;
        Ok(ControlReq {
            client: p.h_client,
            object: p.h_object,
            cmd: p.cmd,
            flags: p.flags,
            params_ptr: p.params,
            params_size: p.params_size,
        })
    }

    /// Decode `NV_ESC_RM_DUP_OBJECT` parameters.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_dup(&self, bytes: &[u8]) -> Result<DupReq, AbiError> {
        let p = nvos::Nvos55Parameters::decode(bytes)?;
        Ok(DupReq {
            dst_client: p.h_client,
            dst_parent: p.h_parent,
            dst_handle: p.h_object,
            src_client: p.h_client_src,
            src_handle: p.h_object_src,
            flags: p.flags,
        })
    }

    /// Decode `NV_ESC_RM_MAP_MEMORY_DMA` parameters **in this version's shape**.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`], with `need` reporting this version's size — so a
    /// 56-byte buffer against a 580.65.06+ table says `need 64, got 56` rather
    /// than succeeding with `dmaOffset` read out of `kindOverride`.
    pub fn decode_map_memory_dma(&self, bytes: &[u8]) -> Result<MapMemoryDma, AbiError> {
        match self.map_dma {
            MapDmaWire::Pre580_65_06 => {
                let p = Nvos46ParametersPre580::decode(bytes)?;
                Ok(MapMemoryDma {
                    client: p.h_client,
                    device: p.h_device,
                    dma: p.h_dma,
                    memory: p.h_memory,
                    offset: p.offset,
                    length: p.length,
                    flags: p.flags,
                    dma_offset: p.dma_offset,
                })
            }
            MapDmaWire::From580_65_06 => {
                let p = nvos::Nvos46Parameters::decode(bytes)?;
                Ok(MapMemoryDma {
                    client: p.h_client,
                    device: p.h_device,
                    dma: p.h_dma,
                    memory: p.h_memory,
                    offset: p.offset,
                    length: p.length,
                    flags: p.flags,
                    dma_offset: p.dma_offset,
                })
            }
        }
    }

    /// Write the two `[OUT]` fields of `NV_ESC_RM_MAP_MEMORY_DMA` back into the
    /// guest's buffer, at **this version's** offsets.
    ///
    /// Only `dmaOffset` and `status` are written. Every other byte — including
    /// the `[IN]` fields, the padding, and (on 580.65.06+) `flags2` and
    /// `kindOverride` — is left exactly as the guest wrote it. That is the
    /// `writeback_bug_pattern` rule: a writer that rewrites the whole struct
    /// hands the caller back whatever the emulator happened to have in the
    /// field, and CUDA reads it as its own.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if the buffer is shorter than this version's
    /// struct.
    pub fn write_map_memory_dma_result(
        &self,
        bytes: &mut [u8],
        dma_offset: u64,
        status: u32,
    ) -> Result<(), AbiError> {
        let (dma_off_at, status_at, size) = match self.map_dma {
            MapDmaWire::Pre580_65_06 => (40usize, 48usize, Nvos46ParametersPre580::SIZE),
            MapDmaWire::From580_65_06 => (48usize, 56usize, nvos::Nvos46Parameters::SIZE),
        };
        let len = bytes.len();
        if len < size {
            return Err(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            });
        }
        let d = dma_offset.to_le_bytes();
        let s = status.to_le_bytes();
        bytes
            .get_mut(dma_off_at..dma_off_at + 8)
            .ok_or(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            })?
            .copy_from_slice(&d);
        bytes
            .get_mut(status_at..status_at + 4)
            .ok_or(AbiError::Truncated {
                c_name: "NVOS46_PARAMETERS",
                need: size,
                got: len,
            })?
            .copy_from_slice(&s);
        Ok(())
    }

    /// Decode `NV_ESC_RM_UNMAP_MEMORY_DMA` parameters.
    ///
    /// Not versioned within the supported range: `NVOS47_PARAMETERS` took its
    /// current 48-byte shape at 550.54.04, which is this crate's floor.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_unmap_memory_dma(&self, bytes: &[u8]) -> Result<UnmapMemoryDma, AbiError> {
        let p = nvos::Nvos47Parameters::decode(bytes)?;
        Ok(UnmapMemoryDma {
            client: p.h_client,
            device: p.h_device,
            dma: p.h_dma,
            memory: p.h_memory,
            flags: p.flags,
            dma_offset: p.dma_offset,
            size: p.size,
        })
    }

    /// Decode the client-root alloc params under the **prefix contract** — see
    /// [`ClientAllocFacts`] for why it is 8 bytes and not 120.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if fewer than 8 bytes are available.
    pub fn decode_client_alloc_facts(&self, bytes: &[u8]) -> Result<ClientAllocFacts, AbiError> {
        if bytes.len() < CLIENT_ALLOC_PREFIX {
            return Err(AbiError::Truncated {
                c_name: classes::Nv0000AllocParameters::C_NAME,
                need: CLIENT_ALLOC_PREFIX,
                got: bytes.len(),
            });
        }
        Ok(ClientAllocFacts {
            h_client: u32_at(bytes, 0)?,
            process_id: u32_at(bytes, 4)?,
        })
    }

    /// Decode the Device alloc params.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`].
    pub fn decode_device_alloc_facts(&self, bytes: &[u8]) -> Result<DeviceAllocFacts, AbiError> {
        let p = classes::Nv0080AllocParameters::decode(bytes)?;
        Ok(DeviceAllocFacts {
            device_id: p.device_id,
            h_client_share: p.h_client_share,
            h_target_client: p.h_target_client,
            h_target_device: p.h_target_device,
            flags: p.flags,
            va_space_size: p.va_space_size,
            va_mode: p.va_mode,
        })
    }

    /// Decode a `NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY` payload.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`]. The struct's tail (`chId`, `subDeviceId`,
    /// `pasid`) is confirmed only by ogkm 610.43.02, so a shorter payload from an
    /// older driver is refused rather than read past — see the generated module's
    /// version caveat.
    pub fn decode_set_page_dir(&self, bytes: &[u8]) -> Result<SetPageDir, AbiError> {
        let p = ctrl::Nv0080CtrlDmaSetPageDirectoryParams::decode(bytes)?;
        Ok(SetPageDir {
            phys_address: p.phys_address,
            num_entries: p.num_entries,
            aperture: PdbAperture::from_flags(p.flags),
            flags: p.flags,
            h_vaspace: p.h_va_space,
        })
    }

    /// Decode a GSP-RPC envelope, validating its guest-written `length`.
    ///
    /// # Errors
    ///
    /// [`AbiError::Truncated`] if the buffer cannot hold the 32-byte envelope;
    /// [`AbiError::RpcSignature`] if the signature word is wrong;
    /// [`AbiError::RpcLength`] if `length` is below the envelope size or beyond
    /// the buffer.
    pub fn decode_rpc_envelope(&self, bytes: &[u8]) -> Result<RpcEnvelope, AbiError> {
        let h = rpc::RpcMessageHeaderV0300::decode(bytes)?;
        if h.signature != RpcEnvelope::SIGNATURE_VALID {
            return Err(AbiError::RpcSignature {
                found: h.signature,
                expected: RpcEnvelope::SIGNATURE_VALID,
            });
        }
        let payload_len = rpc_payload_len(h.length, bytes.len())?;
        Ok(RpcEnvelope {
            header_version: h.header_version,
            signature: h.signature,
            length: h.length,
            function: h.function,
            rpc_result: h.rpc_result,
            rpc_result_private: h.rpc_result_private,
            sequence: h.sequence,
            payload_len,
        })
    }

    /// The payload bytes of an RPC message, i.e. the flexible-array tail.
    ///
    /// # Errors
    ///
    /// Whatever [`Self::decode_rpc_envelope`] returns; the slice itself cannot
    /// fail once the envelope validated, because the envelope's validation is
    /// exactly the bound this uses.
    pub fn rpc_payload<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], AbiError> {
        let env = self.decode_rpc_envelope(bytes)?;
        bytes
            .get(RpcEnvelope::SIZE..RpcEnvelope::SIZE + env.payload_len)
            .ok_or(AbiError::RpcLength {
                declared: env.length,
                available: bytes.len(),
            })
    }
}

/// The bytes [`ClientAllocFacts`] is decoded from — `hClient` and `processID`.
pub const CLIENT_ALLOC_PREFIX: usize = 8;

impl DriverAbi for DriverAbiTable {
    fn version(&self) -> DriverVersion {
        self.version
    }

    /// The alloc-param size for a class, or `None` when this build cannot state
    /// one.
    ///
    /// ★ `NV01_ROOT` / `NV01_ROOT_CLIENT` are deliberately **absent**, and the
    /// absence is the honest answer rather than a gap: `NV0000_ALLOC_PARAMETERS`
    /// is 120 bytes in ogkm 610.43.02 and has **no second oracle** — neither
    /// nvproxy nor the C artifact models it — so its size at 575/580 is
    /// unverified. Reporting 120 would be exactly the guessed size this table
    /// exists to prevent. The client-kind path does not need it: it reads the
    /// 8-byte prefix contract (`decode_client_alloc_facts`).
    ///
    /// `NV01_DEVICE_0` is present because its 56 bytes are confirmed three ways
    /// (ogkm 610.43.02, `gvisor/pkg/abi/nvgpu/classes.go:198-211`, and the C
    /// artifact's `abi_parity_test.go:120`).
    fn alloc_param_size(&self, class: ClassId) -> Option<usize> {
        if class.0 == classes::NV01_DEVICE_0 {
            Some(classes::Nv0080AllocParameters::SIZE)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is ascending and has no duplicate versions — `table_for`'s
    /// reverse scan is only correct if it is.
    #[test]
    fn the_table_is_strictly_ascending() {
        for w in TABLES.windows(2) {
            assert!(
                w[0].version < w[1].version,
                "{:?} !< {:?}",
                w[0].version,
                w[1].version
            );
        }
    }

    /// Selection is "newest entry <= version", asserted at every boundary
    /// including one patch below each one. This is the assertion the C's
    /// major-only key cannot make.
    #[test]
    fn selection_lands_on_the_exact_boundary_not_the_major() {
        let at = |maj, min, pat| {
            table_for(DriverVersion {
                major: maj,
                minor: min,
                patch: pat,
            })
            .expect("in range")
            .map_dma_wire()
        };
        // Exactly the floor.
        assert_eq!(at(550, 54, 4), MapDmaWire::Pre580_65_06);
        // Well inside the pre-580 range.
        assert_eq!(at(575, 51, 2), MapDmaWire::Pre580_65_06);
        // ★ 580.65.05 is a 580 but PRE-boundary. A major-only key gets this
        // wrong; this is the whole reason the table is keyed on all three.
        assert_eq!(at(580, 65, 5), MapDmaWire::Pre580_65_06);
        assert_eq!(at(580, 64, 255), MapDmaWire::Pre580_65_06);
        // Exactly the boundary.
        assert_eq!(at(580, 65, 6), MapDmaWire::From580_65_06);
        // The bench.
        assert_eq!(at(580, 159, 4), MapDmaWire::From580_65_06);
        // Newer than every entry inherits the newest.
        assert_eq!(at(999, 0, 0), MapDmaWire::From580_65_06);
    }

    /// Below the floor is a refusal naming the version — never the nearest
    /// table. (`nvkvm_abi.h:105-110` returns the 570 profile here.)
    #[test]
    fn below_the_floor_is_a_loud_refusal_not_the_nearest_table() {
        for (maj, min, pat) in [
            (550u16, 54u16, 3u16),
            (550, 53, 255),
            (535, 104, 5),
            (0, 0, 0),
        ] {
            assert_eq!(
                table_for(DriverVersion {
                    major: maj,
                    minor: min,
                    patch: pat
                })
                .map(|t| t.version),
                Err(AbiError::NoTableForVersion {
                    major: maj,
                    minor: min,
                    patch: pat
                }),
                "{maj}.{min}.{pat} must not resolve to a table",
            );
        }
    }

    /// The bench driver constant resolves, and resolves to the 64-byte NVOS46.
    /// Non-vacuity for the whole version story: if this ever flips, the crate is
    /// decoding the bench's own traffic wrong.
    #[test]
    fn the_bench_driver_resolves_to_the_64_byte_nvos46() {
        let t = table_for(BENCH_DRIVER).expect("the bench driver is supported");
        assert_eq!(t.map_dma_wire(), MapDmaWire::From580_65_06);
        assert_eq!(t.map_dma_size(), 64);
        assert_eq!(t.map_dma_status_offset(), 56);
    }

    /// The two sizes and the two status offsets, pinned against the C artifact's
    /// own hand-maintained table (`nvkvm_abi.h:66,76,86`).
    #[test]
    fn sizes_and_status_offsets_match_the_c_artifacts_profile_table() {
        let old = table_for(DriverVersion {
            major: 575,
            minor: 51,
            patch: 2,
        })
        .expect("in range");
        assert_eq!(old.map_dma_size(), 56, "nvkvm_abi.h:76 .nvos46_size = 56");
        assert_eq!(
            old.map_dma_status_offset(),
            48,
            "nvkvm_abi.h:76 .nvos46_status_off = 48"
        );
        let new = table_for(DriverVersion {
            major: 580,
            minor: 65,
            patch: 6,
        })
        .expect("in range");
        assert_eq!(new.map_dma_size(), 64, "nvkvm_abi.h:86 .nvos46_size = 64");
        assert_eq!(
            new.map_dma_status_offset(),
            56,
            "nvkvm_abi.h:86 .nvos46_status_off = 56"
        );
    }

    /// `alloc_param_size` states what it knows and refuses what it does not, and
    /// the refusal is asserted for the specific class it is about — so the day
    /// someone populates `NV01_ROOT` from a second oracle, this test changes.
    #[test]
    fn alloc_param_size_reports_only_the_triple_confirmed_class() {
        let t = table_for(BENCH_DRIVER).expect("supported");
        assert_eq!(
            t.alloc_param_size(ClassId(classes::NV01_DEVICE_0)),
            Some(56)
        );
        assert_eq!(
            t.alloc_param_size(ClassId(classes::NV01_ROOT)),
            None,
            "NV0000_ALLOC_PARAMETERS has no second oracle — see the method doc"
        );
        assert_eq!(t.alloc_param_size(ClassId(classes::NV01_ROOT_CLIENT)), None);
        assert_eq!(t.alloc_param_size(ClassId(0xDEAD_BEEF)), None);
    }
}
