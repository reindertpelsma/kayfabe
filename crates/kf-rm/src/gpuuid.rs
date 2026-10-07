//! **STATUS: LIVE, 2026-10-08.** The per-VM GPU UUID: what the kf3 device's `gpu-uuid` property
//! means, and the one function ([`resolve`]) that turns it into the 16 bytes the guest reads.
//!
//! ## Why this exists
//!
//! Orchestrators and cluster tools that key on the GPU id crash or misbehave when two VMs report
//! the same one. Before this module the guest UUID was a function of the chip row
//! ([`crate::staticinfo::StaticInfoPolicy::gid_for_board`]), so every VM on a given chip row
//! reported the SAME UUID. This is a COLLISION problem. It is not about hiding the host GPU
//! (the guest is told the host GPU's model and architecture truthfully; see *Not done* below).
//!
//! ## The modes (`gpu-uuid=`)
//!
//! | value | meaning |
//! |---|---|
//! | unset, empty, `auto` | **default.** [`auto_gid`]: stable across boots of the same VM, distinct across VMs |
//! | `random` | fresh 16 bytes from the OS per boot; never stable |
//! | `host` | the host GPU's own UUID, with a logged warning (two VMs on one GPU then collide) |
//! | `GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, the same without `GPU-`, or 32 hex digits | explicit; refused if malformed or all-zero |
//!
//! ## The `auto` construction (exact)
//!
//! ```text
//! digest = SHA-256( "kayfabe/gpu-uuid/auto/v1\0"   // 25 bytes, domain-separation tag
//!                || vm_id                          // 16 bytes
//!                || host_gid                       // 16 bytes
//!                || slot )                         //  1 byte
//! gid    = digest[0 .. 16]
//! ```
//!
//! Every field is fixed-width, so no input can be re-parsed as another (no length prefix is
//! needed). A different tag version is a different identity space; the tag is never edited in
//! place, a `v2` is added.
//!
//! * `vm_id` — the VM's identity: the `vm-id` device property if set, else QEMU's `-uuid`. An
//!   unset `-uuid` reads as all-zero, which is NOT an identity: see [`Basis::RandomNoVmId`].
//! * `host_gid` — the host GPU's UUID, asked of the host RM at realize with
//!   `NV2080_CTRL_CMD_GPU_GET_GID_INFO` (unprivileged; `HostFacts::host_gid`). Derived, never
//!   captured. It is what makes two GPUs of one VM distinct from each other.
//! * `slot` — the device's guest PCI `devfn` byte. It separates two kf3 devices of one VM that
//!   sit on the SAME host GPU (they share `vm_id` and `host_gid`). ⚠ Moving the device to
//!   another slot therefore changes its UUID; `gpu-uuid=<explicit>` pins it.
//!
//! The owner's earlier formula was `hash(vm id + host GPU UUID)`; `slot` is the one addition,
//! and it is a single byte that can be dropped in [`auto_gid`] if the owner rules a within-VM
//! collision acceptable.
//!
//! ## Not done here
//!
//! * The optional HMAC with an overridable key (owner's design): `auto` is a plain domain-
//!   separated hash, so anyone who knows the VM id and the host GPU UUID can recompute it. Nothing
//!   secret is claimed or protected by it.
//! * The user-settable GPU NAME (architecture and model stay truthful): untouched.
//! * Translating a guest-supplied UUID back to the host's: no v3 path carries one.

use kf_abi::gspstaticinfo::{GpuGid, GpuGidTextError, RM_SHA1_GID_SIZE, parse_uuid_text};

/// The domain-separation tag of the `auto` construction (see the module doc). Never edited in
/// place: a change of construction takes a new version.
pub const AUTO_TAG: &[u8; 25] = b"kayfabe/gpu-uuid/auto/v1\0";

/// What `gpu-uuid=` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuUuidMode {
    /// The default: [`auto_gid`].
    Auto,
    /// A fresh value per boot.
    Random,
    /// The host GPU's own UUID.
    Host,
    /// An operator-declared value (non-zero by construction).
    Explicit(GpuGid),
}

impl GpuUuidMode {
    /// Parse the property. `None`, `""` and `"auto"` are [`GpuUuidMode::Auto`]; the words are
    /// exact (lowercase); anything else must be a UUID text ([`GpuGid::parse`]).
    ///
    /// # Errors
    /// [`GpuUuidError::BadMode`], naming the property and what is accepted.
    pub fn parse(text: Option<&str>) -> Result<GpuUuidMode, GpuUuidError> {
        match text {
            None | Some("" | "auto") => Ok(GpuUuidMode::Auto),
            Some("random") => Ok(GpuUuidMode::Random),
            Some("host") => Ok(GpuUuidMode::Host),
            Some(other) => GpuGid::parse(other)
                .map(GpuUuidMode::Explicit)
                .map_err(|why| GpuUuidError::BadMode {
                    value_len: other.len(),
                    why,
                }),
        }
    }
}

impl GpuUuidMode {
    /// The mode's property word (`explicit` for a declared value) — for the realize log.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            GpuUuidMode::Auto => "auto",
            GpuUuidMode::Random => "random",
            GpuUuidMode::Host => "host",
            GpuUuidMode::Explicit(_) => "explicit",
        }
    }
}

/// Why a `gpu-uuid` / `vm-id` declaration cannot be honoured. Every message names the property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuUuidError {
    /// `gpu-uuid=` is none of `auto`, `random`, `host` or a UUID text. (The value is not echoed:
    /// it is operator input of arbitrary length; only its length is.)
    BadMode {
        /// Length in bytes of the refused value.
        value_len: usize,
        /// Why it is not a UUID.
        why: GpuGidTextError,
    },
    /// `vm-id=` is not a UUID text.
    BadVmId {
        /// Why.
        why: GpuGidTextError,
    },
    /// `auto` or `host` needs the host GPU's UUID and the host RM did not give one.
    HostUuidUnavailable {
        /// Which mode needed it.
        mode: &'static str,
    },
    /// The OS entropy source failed (`random`, or the `auto` fallback to it).
    Entropy(String),
    /// The 16 bytes came out all-zero (probability 2^-128; refused rather than served).
    ZeroResult,
}

impl core::fmt::Display for GpuUuidError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GpuUuidError::BadMode { value_len, why } => write!(
                f,
                "gpu-uuid= ({value_len} bytes) must be auto, random, host, or a UUID \
                 (GPU-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx, the same without GPU-, or 32 hex digits): {why}"
            ),
            GpuUuidError::BadVmId { why } => write!(
                f,
                "vm-id= must be a UUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx or 32 hex digits): {why}"
            ),
            GpuUuidError::HostUuidUnavailable { mode } => write!(
                f,
                "gpu-uuid={mode} needs the host GPU's UUID and the host RM did not answer \
                 NV2080_CTRL_CMD_GPU_GET_GID_INFO; set gpu-uuid=random or an explicit UUID"
            ),
            GpuUuidError::Entropy(why) => write!(f, "gpu-uuid: no OS entropy: {why}"),
            GpuUuidError::ZeroResult => f.write_str("gpu-uuid: the derived UUID is all-zero"),
        }
    }
}

impl std::error::Error for GpuUuidError {}

/// Parse a VM identity text (`vm-id=`, or QEMU's `-uuid` as QEMU prints it).
///
/// Returns `Ok(None)` for `None`/empty and for the all-zero UUID — QEMU's value when `-uuid` was
/// not given, which identifies no VM.
///
/// # Errors
/// [`GpuUuidError::BadVmId`].
pub fn parse_vm_id(text: Option<&str>) -> Result<Option<[u8; RM_SHA1_GID_SIZE]>, GpuUuidError> {
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let id = parse_uuid_text(text).map_err(|why| GpuUuidError::BadVmId { why })?;
    Ok(Some(id).filter(|id| id.iter().any(|b| *b != 0)))
}

/// The `auto` UUID: see the module doc for the exact construction.
///
/// # Errors
/// [`GpuUuidError::ZeroResult`] (2^-128).
pub fn auto_gid(
    vm_id: &[u8; RM_SHA1_GID_SIZE],
    host_gid: &GpuGid,
    slot: u8,
) -> Result<GpuGid, GpuUuidError> {
    let mut msg = [0u8; AUTO_TAG.len() + 2 * RM_SHA1_GID_SIZE + 1];
    let (tag, rest) = msg.split_at_mut(AUTO_TAG.len());
    tag.copy_from_slice(AUTO_TAG);
    rest[..RM_SHA1_GID_SIZE].copy_from_slice(vm_id);
    rest[RM_SHA1_GID_SIZE..2 * RM_SHA1_GID_SIZE].copy_from_slice(host_gid.as_bytes());
    rest[2 * RM_SHA1_GID_SIZE] = slot;
    let digest = kf_util::sha256::sha256(&msg);
    let mut out = [0u8; RM_SHA1_GID_SIZE];
    out.copy_from_slice(&digest[..RM_SHA1_GID_SIZE]);
    GpuGid::from_bytes(out).ok_or(GpuUuidError::ZeroResult)
}

/// How the UUID came to be — logged at realize, and asserted by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// [`auto_gid`] over the VM id, the host GPU's UUID and the slot.
    Auto,
    /// `random`.
    Random,
    /// `host`.
    Host,
    /// An operator-declared value.
    Explicit,
    /// `auto` was asked and the VM has no identity (no `vm-id`, no `-uuid`): there is nothing
    /// stable to derive from, so the value is `random`. ⚠ Collision-free, but it changes every
    /// boot. See the owner decision in the commit message / design doc.
    RandomNoVmId,
}

/// The resolved UUID and what the operator should be told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The ONE guest-visible UUID.
    pub gid: GpuGid,
    /// How it was obtained.
    pub basis: Basis,
    /// Lines for the realize log, each already a complete sentence.
    pub warnings: Vec<String>,
}

/// What [`resolve`] decides from.
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    /// The `gpu-uuid=` property.
    pub mode: GpuUuidMode,
    /// The VM's identity (`vm-id=` or `-uuid`), if it has one ([`parse_vm_id`]).
    pub vm_id: Option<[u8; RM_SHA1_GID_SIZE]>,
    /// The device's guest PCI `devfn`.
    pub slot: u8,
    /// The host GPU's UUID, as the host RM reported it; `None` if it did not.
    pub host_gid: Option<GpuGid>,
}

type Entropy<'a> = &'a mut dyn FnMut(&mut [u8; RM_SHA1_GID_SIZE]) -> Result<(), String>;

fn random_gid(entropy: Entropy<'_>) -> Result<GpuGid, GpuUuidError> {
    let mut b = [0u8; RM_SHA1_GID_SIZE];
    entropy(&mut b).map_err(GpuUuidError::Entropy)?;
    GpuGid::from_bytes(b).ok_or(GpuUuidError::ZeroResult)
}

/// Resolve the property to the guest-visible UUID.
///
/// `entropy` fills 16 bytes from the OS (injected so tests are deterministic); it is called only
/// for `random` and for the no-VM-identity fallback of `auto`.
///
/// # Errors
/// [`GpuUuidError`]: `auto`/`host` without a host UUID, an entropy failure, an all-zero result.
pub fn resolve(
    inp: &Inputs,
    entropy: Entropy<'_>,
) -> Result<Resolved, GpuUuidError> {
    match inp.mode {
        GpuUuidMode::Explicit(gid) => Ok(Resolved {
            gid,
            basis: Basis::Explicit,
            warnings: Vec::new(),
        }),
        GpuUuidMode::Random => Ok(Resolved {
            gid: random_gid(entropy)?,
            basis: Basis::Random,
            warnings: Vec::new(),
        }),
        GpuUuidMode::Host => {
            let gid = inp
                .host_gid
                .ok_or(GpuUuidError::HostUuidUnavailable { mode: "host" })?;
            Ok(Resolved {
                gid,
                basis: Basis::Host,
                warnings: vec![
                    "gpu-uuid=host: the guest reports the HOST GPU's own UUID. Two VMs on this GPU, \
                     or a VM and a host tool, then report the same id; use auto for a per-VM one."
                        .into(),
                ],
            })
        }
        GpuUuidMode::Auto => {
            let Some(vm_id) = inp.vm_id else {
                return Ok(Resolved {
                    gid: random_gid(entropy)?,
                    basis: Basis::RandomNoVmId,
                    warnings: vec![
                        "gpu-uuid=auto: this VM has no identity (no vm-id= and no QEMU -uuid), so \
                         there is nothing stable to derive from; using a RANDOM UUID for this boot. \
                         Give the VM `-uuid <uuid>` (or vm-id=) for a UUID that is stable across \
                         boots."
                            .into(),
                    ],
                });
            };
            let host = inp
                .host_gid
                .ok_or(GpuUuidError::HostUuidUnavailable { mode: "auto" })?;
            Ok(Resolved {
                gid: auto_gid(&vm_id, &host, inp.slot)?,
                basis: Basis::Auto,
                warnings: Vec::new(),
            })
        }
    }
}

/// Fill `buf` from the operating system's entropy (`/dev/urandom`).
///
/// # Errors
/// The I/O error, as text.
pub fn os_entropy(buf: &mut [u8; RM_SHA1_GID_SIZE]) -> Result<(), String> {
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .map_err(|e| format!("/dev/urandom: {e}"))
}
