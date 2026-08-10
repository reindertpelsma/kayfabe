/*
 * kayfabe_shim.h — the ENTIRE contract between this C device shim and the Rust archive.
 *
 * ★★★ This file names no hypervisor type, on purpose.  It is `stdint.h` and nothing else,
 * because the seam it describes is the one place a second hypervisor plugs in: a Cloud
 * Hypervisor device fills the same table with its own primitives and links the same archive.
 * If a `MemoryRegion *` or an `Error **` ever appears below this line, the seam has moved to
 * the wrong side and a second hypervisor has become a bolt-on.
 *
 * ★★ It is hand-mirrored in `crates/kayfabe-qemu-raw/src/shim_unsafe.rs`.  That is a real
 * hazard and it is answered rather than hoped away: every structure carries an ABI number AND
 * its own `sizeof`, both checked at realize, so a field added on one side only is a named
 * refusal at startup instead of a jump through a garbage address later.
 */

#ifndef KAYFABE_SHIM_H
#define KAYFABE_SHIM_H

#include <stdint.h>

/* Bump on ANY change to the structures or the meaning of a status code. */
#define KAYFABE_SHIM_ABI 36u

/*
 * Status classes.  ★ The negative convention is load-bearing: a return value below zero is
 * `-errno`, so a primitive can hand the operating system's own number up without inventing a
 * class for it, and KAYFABE_E_REFUSED means "refused, and there was no number".
 */
#define KAYFABE_OK             0
#define KAYFABE_E_REFUSED      1
#define KAYFABE_E_BUSY         2   /* a conflicting REQUIRER is present (§8.5's -EBUSY) */
#define KAYFABE_E_UNSUPPORTED  3
#define KAYFABE_E_MALFORMED    4   /* the CALL was wrong, not the machine */

/*
 * The primitives the Rust archive needs from its host.
 *
 * ★ Every entry must be non-NULL: the archive checks and refuses by name if one is missing,
 * rather than discovering it by calling address zero.  The obligations each carries are
 * stated on it; three are load-bearing for correctness rather than tidiness and are marked
 * NORMATIVE.
 */
/* ★ Every structure in this header closes with `} Name;` on one line, and that is not a
 * style preference — the Axis-A gate re-derives the own-wire proof by grepping for exactly
 * that form. This one used the forward-typedef spelling until the gate refused it, which is
 * the instrument working: a shape the proof cannot see is a shape the proof does not cover. */
typedef struct KayfabeHostOps {
    uint32_t abi_version;   /* == KAYFABE_SHIM_ABI      */
    uint32_t struct_size;   /* == sizeof(KayfabeHostOps) */

    /* The hypervisor BINARY's version.  See nvkvm.c for why, for an in-tree device, this is
     * the same fact the compile-time floor asserts — and why that is not a defect. */
    uint32_t (*version_major)(void *dev);
    uint32_t (*version_minor)(void *dev);

    /* Non-zero if this machine runs on the hardware accelerator. */
    int32_t (*kvm_enabled)(void *dev);

    /* Block migration.  `reason` is (pointer, length) and is NOT NUL-terminated; the callee
     * must copy it and must not retain the pointer. */
    int32_t (*migrate_add_blocker)(void *dev, const uint8_t *reason, uint64_t reason_len,
                                   uint64_t *out_id);
    void    (*migrate_del_blocker)(void *dev, uint64_t id);

    /* Refuse guest-driven discard of RAM ranges machine-wide.  MUST return KAYFABE_E_BUSY,
     * not -EBUSY, when a discard REQUIRER is already present: the archive reports that arm
     * differently from every other refusal because it is an operator's configuration mistake
     * and not a bug. */
    int32_t (*ram_block_discard_disable)(void *dev, int32_t disable);

    /* Arrange for kayfabe_shim_region_add/_del to be called for the address space this device
     * does DMA in.  ★ The callbacks MAY NOT run before this call returns — the archive is
     * still inside realize and has no handle to give them yet. */
    int32_t (*register_listener)(void *dev);

    /* ★★★ NORMATIVE.  Non-zero if `bar` is a PURE-MMIO reservation the hypervisor does not
     * back.  The archive installs its own accelerator slots over that guest-physical range,
     * and the entire safety argument for doing so is that the hypervisor's own listener takes
     * an early return for a non-RAM region and therefore never creates, deletes or looks up a
     * slot of its own there.  A shim that registers the register with a RAM-backed
     * constructor and answers non-zero here has put two slots over one range with only one
     * winner. */
    int32_t (*bar_is_unbacked_reservation)(void *dev, uint32_t bar);

    /* ★★★ NORMATIVE.  Where `bar` is CURRENTLY programmed.  Returns KAYFABE_OK and writes the
     * base, or KAYFABE_E_UNSUPPORTED while it is unmapped.  This is on the hot path and MUST
     * be one read of the device's own PCI bookkeeping — never a call that takes the
     * hypervisor's global lock.  It is re-read rather than cached because a base cached once
     * is silently wrong the moment the guest moves the register. */
    int32_t (*bar_base)(void *dev, uint32_t bar, uint64_t *out_base);

    /* Counted references to regions the listener reported. */
    int32_t (*ref_region)(void *dev, uint64_t mr);
    void    (*unref_region)(void *dev, uint64_t mr);

    /* ★★★ NORMATIVE.  A BOUNDED copy against THIS region's own backing.  It must not be
     * spelled as a general read/write-anywhere accessor: that entry point takes the
     * hypervisor's global lock whenever the target is not direct-access, which would put a
     * foreign lock underneath one of the archive's ranked locks.  Copy no more than `len`
     * bytes and refuse rather than truncate. */
    int32_t (*read_region)(void *dev, uint64_t mr, uint64_t off, uint8_t *dst, uint64_t len);
    int32_t (*write_region)(void *dev, uint64_t mr, uint64_t off, const uint8_t *src,
                            uint64_t len);

    /* One descriptor write.  Never a notify call that takes the global lock. */
    int32_t (*signal_msix)(void *dev, uint16_t vector);
} KayfabeHostOps;

/* One PCI base-address register, as the shim's realize-time region table describes it. */
typedef struct KayfabeBarCfg {
    uint32_t index;
    uint32_t reserved;
    uint64_t base;
    uint64_t len;
} KayfabeBarCfg;

/* What realize is asked for. */
typedef struct KayfabeRealizeCfg {
    uint32_t abi_version;   /* == KAYFABE_SHIM_ABI          */
    uint32_t struct_size;   /* == sizeof(KayfabeRealizeCfg) */
    int32_t  shareable_ram;
    uint32_t n_bars;
    const KayfabeBarCfg *bars;
} KayfabeRealizeCfg;

/*
 * One topology section, UNCLASSIFIED.
 *
 * ★★ Five facts, not one predicate, because "is this memory?" is wrong in three independent
 * directions and no single accessor answers it.  The shim reports what it sees; the rule that
 * turns five facts into a verdict lives in exactly one place, and it is not here.
 */
typedef struct KayfabeSection {
    uint64_t mr;
    uint64_t gpa;
    uint64_t len;
    uint64_t offset_within_region;
    int32_t  is_ram;
    int32_t  is_ram_device;
    int32_t  is_rom_device;
    int32_t  readonly;
    int32_t  nonvolatile;
} KayfabeSection;

/* Counters, so an acceptance test outside the process can assert on more than an exit code.
 * u64-only and address-free by construction. */
typedef struct KayfabeAudit {
    uint64_t live_windows;
    uint64_t live_memslots;
    uint64_t memslot_installs;
    uint64_t regions_published;   /* ★ must be zero forever */
    uint64_t topology_adds;
    uint64_t topology_dels;
    uint64_t bar_base_checks;
    uint64_t bar_moves_detected;
    uint64_t ops_refused_after_unrealize;
} KayfabeAudit;

/* ---- the entry points the archive exports ------------------------------------------ */

/* The wire ABI the archive speaks.  The one call that takes no address, so it is safe to make
 * first, before anything has been validated. */
uint32_t kayfabe_shim_abi_version(void);

/* ★★★★ §16.65 — the LABEL for bucket `idx` of KayfabeRegAudit::doorbells_by_engine.
 *
 * ⊘ The names cross the seam rather than being written down here, because a C-side table
 * would be a SECOND declaration of one ordering: it would keep printing every bucket, with
 * a plausible number under the wrong name, and nothing would catch it.  The order is
 * `kayfabe_rt::EngineKind::ALL`'s, once, and this asks for it.
 *
 * Returns a static, NUL-terminated string the caller must not free; an out-of-range index
 * yields "?" and never NULL, because a NULL handed to %s is undefined behaviour in the
 * caller and a refusal must not be worse than what it refuses. */
const char *kayfabe_shim_engine_kind_name(uint32_t idx);

/* Realize the memory plane.  On KAYFABE_OK, *out_handle is the handle every other entry point
 * takes.  On any other return *out_handle is untouched and (*out_msg, *out_msg_len) describe
 * the refusal — the text is static storage inside the archive and may be held indefinitely.
 * It is NOT NUL-terminated: print it with "%.*s". */
int32_t kayfabe_shim_realize(const KayfabeHostOps *ops, void *dev,
                             const KayfabeRealizeCfg *cfg, void **out_handle,
                             const uint8_t **out_msg, uint64_t *out_msg_len);

void    kayfabe_shim_unrealize(void *handle);

int32_t kayfabe_shim_region_add(void *handle, const KayfabeSection *section,
                                const uint8_t **out_msg, uint64_t *out_msg_len);
void    kayfabe_shim_region_del(void *handle, uint64_t gpa, uint64_t len);

/* The preventer: call BEFORE letting a base-address-register write through. */
int32_t kayfabe_shim_bar_move_requested(void *handle, uint32_t bar,
                                        const uint8_t **out_msg, uint64_t *out_msg_len);
/* The detector: call AFTER one has gone through. */
int32_t kayfabe_shim_note_bar_mapping(void *handle, uint32_t bar, int32_t mapped,
                                      uint64_t base);

int32_t kayfabe_shim_install_window(void *handle, uint64_t gpa, uint64_t len,
                                    const uint8_t **out_msg, uint64_t *out_msg_len);

int32_t kayfabe_shim_audit(void *handle, KayfabeAudit *out);

/* ═══════════════════════════════════════════════════════════════════════════════════════
 * THE REGISTER PLANE (stage Q4) — a SECOND handle, deliberately
 * ═══════════════════════════════════════════════════════════════════════════════════════
 *
 * ★★★ Why this is not part of the memory plane's handle, which is the obvious shape.
 *
 * The memory plane realizes LATE — it needs base-address registers, and firmware programs
 * those long after the device object exists — and it may refuse outright, at which point
 * the device is deliberately dead.  The register plane needs neither: a guest driver's very
 * first act is to read chip-identity registers, and the answer is a function of the chip
 * table and nothing else.  Hanging the registers off the memory plane's handle would mean a
 * device whose registers answer zero until firmware has finished, and a device whose memory
 * plane refused could not tell a driver *anything* — including the refusal.
 *
 * So: two handles, two lifetimes, one device.  The register plane is created at realize and
 * destroyed at exit.  Joining them (so the GSP's queue doorbell can reach guest RAM) is the
 * NEXT stage and is a named refusal today — see `KayfabeRegAudit::ram_refusals`.
 */

/* What a chip's device must put in configuration space before a stock driver will bind.
 *
 * ★ Field order is fixed for natural alignment on both sides (8-byte, then 4, then 2, then
 * 1) so the two spellings of this structure cannot differ by padding.  `struct_size` is
 * checked anyway. */
typedef struct KayfabeChipIdentity {
    uint32_t abi_version;        /* == KAYFABE_SHIM_ABI            */
    uint32_t struct_size;        /* == sizeof(KayfabeChipIdentity) */
    uint64_t regs_aperture_len;  /* the register BAR's size, per the chip table */
    /* ★★ The two windows' sizes, per the SAME chip table row the emulated GSP answers
     * NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO from.  They are here so this device cannot
     * register an aperture of one size while telling the guest's resource manager
     * another: RM copies barSizeBytes straight into pKernelBus->pciBarSizes[] and sizes
     * its own mappings against it, and a disagreement never logs. */
    uint64_t fb_window_len;
    uint64_t inst_window_len;
    uint32_t class_code;         /* (base << 16) | (sub << 8) | prog_if */
    uint16_t vendor_id;
    uint16_t device_id;
    uint16_t subsystem_vendor_id;
    uint16_t subsystem_id;
    uint16_t msix_vectors;
    uint8_t  revision;
    uint8_t  reserved;
} KayfabeChipIdentity;

/* What one register WRITE did.  A read needs none of this and returns its value directly.
 *
 * `fault` is (pointer, length) into the archive's read-only data, or NULL.  It is NOT
 * NUL-terminated; print it with "%.*s". */
typedef struct KayfabeRegWrite {
    const uint8_t *fault;
    uint64_t       fault_len;
    /* ★★ Stage Q5.  WHY a guest-RAM access was refused, in the port's own words, or NULL.
     * Also (pointer, length), also NOT NUL-terminated.
     *
     * Non-NULL exactly when the fault was a guest-RAM refusal — the only fault that has an
     * address — so it doubles as the validity flag for `ram_gpa` and `ram_len`, which have
     * no reserved value of their own: guest-physical address zero is an ordinary address,
     * and a length of zero is a legal access. */
    const uint8_t *ram_why;
    uint64_t       ram_why_len;
    uint64_t       ram_gpa;            /* valid only when ram_why != NULL */
    uint64_t       ram_len;            /* valid only when ram_why != NULL */
    /* ★★★ #146 THE BAR0 MOVING WINDOW.  Exactly the shape of the four fields above, one
     * aperture over: why a framebuffer write through NV_PBUS_BAR0_WINDOW was refused, and
     * where.  Non-NULL means THE BYTES DID NOT LAND.
     *
     * There is deliberately no third answer.  kbusInitBar2 programs this window and never
     * reads any of it back, so a window that silently dropped writes let every earlier step
     * return NV_OK and was caught only at kbusVerifyBar2, hundreds of operations later, as
     * NV_ERR_MEMORY_ERROR.  These fields are what makes a dropped write loud IN THE SAME
     * BOOT, at the instant it happens. */
    const uint8_t *fb_why;
    uint64_t       fb_why_len;
    uint64_t       fb_phys;            /* valid only when fb_why != NULL */
    uint64_t       fb_refused_len;     /* valid only when fb_why != NULL */
    /* Where a write LANDED, and its own validity flag.
     *
     * ⚠ Two fields for one fact, and the second is not redundant: framebuffer address ZERO
     * is where a window at its reset base points, i.e. exactly where a boot that never
     * programmed the window would write.  A single field could not tell "landed at 0" from
     * "did not land". */
    uint64_t       fb_landed;
    int32_t        fb_landed_valid;
    uint32_t       transitions;
    uint32_t       commands;
    int32_t        claimed;            /* the register model owns this offset */
    int32_t        raise_status_irq;   /* the status queue wants announcing */
    /* ★★★ #151.  The guest wrote NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER and a
     * vector is now pending in the CPU interrupt tree: DELIVER a message-signalled
     * interrupt.
     *
     * ⚠ A second flag beside raise_status_irq and deliberately not the same one, though
     * today both would end in one msix_notify().  They are two different CAUSES — the
     * emulated GSP announcing that it queued something, versus the guest asking the device
     * to interrupt the guest — and a boot that delivered the wrong number of vectors has to
     * be diagnosable as to which.  Only this one is delivered at this stage; see
     * nvkvm_trap_write. */
    int32_t        raise_cpu_intr;
    /* ★★★ E2 — THIS WRITE WAS A USERMODE DOORBELL, and here is what the core answered.
     *
     * `doorbell` is 0 when the write was NOT a doorbell at all — which is the CONTROL every
     * E2 acceptance needs, in the wire shape rather than as a counter comparison.  1 means
     * the core SERVED it (a DoorbellOutcome came back); 2 means the core REFUSED it BY NAME
     * and `doorbell_kind` is that name.
     *
     * ⊘ There is deliberately no third value and no "accepted, dropped" state: a ring that
     * was counted and went nowhere while looking healthy is the exact shape #146 had to
     * remove from the framebuffer write path.
     *
     * ★★ WHY THIS IS ON THE PER-WRITE STRUCT AND THE ISOLATE CENSUS IS NOT.  An isolate
     * refusal is a property of a whole boot.  A doorbell is a property of ONE WRITE, and
     * E2's acceptance is that THIS guest store, at THIS instant, reached the core — so the
     * shell logs it as it happens, against QEMU's own -msg timestamp, i.e. against a
     * timeline the device under test does not write.  A per-boot counter cannot be stamped.
     *
     * `doorbell_kind` is (pointer, length) into the archive's read-only data, or NULL, like
     * `fault`.  It is a FaultTag — a &'static str from a fixed finite set — never a
     * formatted string, so no host allocation crosses here.  Print it with "%.*s". */
    int32_t        doorbell;           /* 0 = not a doorbell, 1 = served, 2 = REFUSED */
    uint64_t       doorbell_token;     /* valid only when doorbell != 0 */
    const uint8_t *doorbell_kind;      /* non-NULL only when doorbell == 2 */
    uint64_t       doorbell_kind_len;
} KayfabeRegWrite;

/* What KayfabeRegWrite::doorbell may be. */
#define KAYFABE_DOORBELL_NONE    0
#define KAYFABE_DOORBELL_SERVED  1
#define KAYFABE_DOORBELL_REFUSED 2
/* ★★★ E10e — the SHELL served it, on the CPU, with no host ring involved: a GSP-managed
 * copy-engine channel (RM's CeUtils) whose operands live in the emulated framebuffer and in
 * guest RAM, neither of which a real engine can be pointed at.  `doorbell_kind` carries the
 * constant name "CpuCe::ServedLocally"; WHAT the executor did reaches this shell once, at
 * teardown, through KayfabeRegAudit::doorbell_local_serving.
 *
 * ⊘ A fourth value rather than reusing _SERVED, because the two are different events with
 * different evidence — one rang a host channel, the other moved bytes with the CPU — and a
 * report that cannot tell forwarding from emulation is the one thing this device's evidence
 * must never do. */
#define KAYFABE_DOORBELL_SERVED_LOCAL 3

/* Register-plane counters.  u64-only and address-free, like KayfabeAudit.
 *
 * ★ `unclaimed_reads` is the honest one: it counts every register this device answered
 * with a DEFAULTED ZERO because no model owns the offset.  It is not an error today (the
 * C artifact does the same, and refusing would mean the device could not boot until every
 * register in a 16 MiB aperture had a model) — it is the number that says how much of a
 * boot rests on that. */

/* How many distinct unserviced commands KayfabeRegAudit carries, and the low half of a
 * packed entry that names no control.
 *
 * ⊘⊘ 32 -> 64, and the width is the SMALLER half of the change.  [measured 2026-08-09] boot
 * gt1431_ff7a0ea printed "67 UNSERVICED ..., 32 distinct" from a list that was SATURATED at
 * 32, because unserviced_len was filled from the sample's own clamped length and could not
 * exceed the array.  A full list therefore read exactly like a complete one, and
 * execution_plane_increments.md §14.31 concluded from a resulting miss that a control
 * "never reaches the emulated GSP".  It does.  unserviced_len is now the TRUE distinct
 * count, and the printer says so out loud when it exceeds the array. */
#define KAYFABE_UNSERVICED_SLOTS 64u
#define KAYFABE_UNSERVICED_NO_CMD 0xFFFFFFFFu

/* How many distinct BRIDGE-REFUSAL tags KayfabeRegAudit carries, and how many bytes of
 * each tag's name it holds.  The name crosses BY VALUE rather than as a pointer: the Rust
 * side's host-pointer gate forbids a host address outside its *_unsafe.rs files, and
 * passing one as an integer would defeat that gate rather than satisfy it. */
#define KAYFABE_BRIDGE_REFUSAL_SLOTS 32u
#define KAYFABE_BRIDGE_REFUSAL_TAG_LEN 64u

/* ★★★★ 16.56 — how many refused IDENTIFIERS each tag row carries.  MUST equal
 * `kayfabe_qemu_raw::shim::REFUSAL_IDS_PER_TAG` and `kayfabe_rmrpc::REFUSAL_DETAIL_CAP`.
 *
 * ⊘ A FaultTag is a &'static str, so a refusal ABOUT A VALUE — an hClass, a control cmd —
 * lost that value the instant it became a census key.  [measured 2026-08-10, over
 * traces/guest_boots/*_qemu.log] `grep -c hClass` over every committed device log returns
 * ZERO: this port had never once named a class it refused, and answering "which ones?"
 * meant reading the GUEST's dmesg, a plane we neither own nor always capture. */
/* ★★★★ §16.65 — how many engine buckets the doorbell census has.  Must equal
 * `kayfabe_qemu_raw::ENGINE_KINDS` and `kayfabe_rt::ENGINE_KIND_COUNT`. */
#define KAYFABE_ENGINE_KINDS 6u

#define KAYFABE_REFUSAL_IDS_PER_TAG 8u

typedef struct KayfabeBridgeRefusal {
    uint8_t tag[KAYFABE_BRIDGE_REFUSAL_TAG_LEN];  /* NUL-PADDED, not NUL-terminated */
    uint64_t tag_len;
    uint64_t count;
    /* Ascending; entries at or past `ids_len` carry no meaning.  `ids_len` is CAPPED and
     * `count` is NOT, so `n` ids beside a larger count reads as a visible truncation. */
    uint32_t ids[KAYFABE_REFUSAL_IDS_PER_TAG];
    uint64_t ids_len;
} KayfabeBridgeRefusal;

/* How many bytes of the isolate plane's refusal SENTENCE KayfabeRegAudit carries, and the
 * three values its `kind` may take.  Longer than a bridge tag because it is not a tag: a
 * spawn failure's text is formatted from the host's own error at the failing step, and
 * truncating it to a tag width would cut off exactly the errno an operator acts on.
 *
 * ★ NONE is 0 so that an all-zero audit reads as "nothing refused", which is true of a
 * struct nobody wrote; the two real kinds are non-zero so an unwritten struct can never be
 * read as a specific diagnosis. */
#define KAYFABE_ISOLATE_REFUSAL_LEN 192u
#define KAYFABE_ISOLATE_REFUSAL_NONE 0u
#define KAYFABE_ISOLATE_REFUSAL_NO_PLANE 1u
#define KAYFABE_ISOLATE_REFUSAL_SPAWN_FAILED 2u

/* ★★★ E2 — how many bytes of a refused doorbell's KIND and SENTENCE the audit carries.
 *
 * Two arrays and not one, for the reason KayfabeIsolateRefusal separates its kind from its
 * text: the kind is a stable name a check may branch on (FwdFault::MalformedToken is a
 * different diagnosis from FwdFault::UnknownVchid, with a different fix), and the sentence
 * is the fault variant's payload, which is prose.  A single blob would make the only
 * machine-readable half a substring search. */
#define KAYFABE_DOORBELL_KIND_LEN 64u
/* ★★ 448 -> 1024 at execution_plane_increments.md §16.6, and the widening is the SMALLER
 * half of the change.  MEASURED 2026-08-09 (boot vaspan_994bbdc) the refusal sentence was
 * 292 bytes of the 448, and §16.6 appends the deciding VA space's whole publication body —
 * four PdeLevels, ~180 bytes — to the END of it, i.e. ~472 into a 448-byte array.  The Rust side used to fill this with a
 * bare min(), so a clipped sentence and a complete one printed IDENTICALLY and the levels
 * would have been the first thing lost.  It now stamps a literal " [CLIPPED, sentence was
 * N bytes]" tail, so saturation is a statement rather than an absence. */
/* ★ 1024 -> 2048 at §16.8.  MEASURED (boot row1_44b7d69): the sentence that boot emitted is
 * 502 bytes, so the 448 this replaced would have cut 54 bytes — L2= and L3=, half of §16.8's
 * finding — off the end SILENTLY.  §16.8's framebuffer dump adds ~380 bytes on the good path
 * and up to ~760 on the refusing one, because a level dump carries the FB store's own
 * sentence and OUTSIDE_FRAMEBUFFER alone is ~190 bytes.  ⊘ Sized against the refusing path:
 * a diagnostic that fits only when nothing went wrong clips exactly when it is read. */
#define KAYFABE_DOORBELL_REFUSAL_LEN 2048u

/* ★★★★ §16.40 — how many bytes of the promote-ctx diagnosis cross the ABI.  MUST equal
 * `kayfabe_qemu_raw::shim::PROMOTE_DIAG_LEN`; the pair is what `KAYFABE_SHIM_ABI` guards. */
#define KAYFABE_PROMOTE_DIAG_LEN 2048u

/* ★★★★ §16.40 — how many promote-ctx refusal KINDS cross.  `PromoteFault` has ten variants,
 * so this is bounded by a FIXED FINITE SET and never by anything the guest supplies. */
#define KAYFABE_PROMOTE_DIAG_SLOTS 4u

/* ★★★★ §16.40 — one promote-ctx refusal KIND, with the address plane's state at the first
 * refusal carrying it.  Both arrays are NUL-PADDED, not NUL-terminated: print with an
 * explicit precision taken from the matching `_len`, never with %s. */
typedef struct KayfabePromoteDiag {
    uint8_t  tag[KAYFABE_BRIDGE_REFUSAL_TAG_LEN];
    uint64_t tag_len;
    uint8_t  text[KAYFABE_PROMOTE_DIAG_LEN];
    uint64_t text_len;
} KayfabePromoteDiag;

typedef struct KayfabeDoorbellRefusal {
    uint8_t kind[KAYFABE_DOORBELL_KIND_LEN];  /* NUL-PADDED, not NUL-terminated */
    uint8_t text[KAYFABE_DOORBELL_REFUSAL_LEN];
    uint64_t kind_len;
    uint64_t len;
    /* ⊘ Non-zero exactly when a doorbell was refused, and the validity flag for everything
     * above.  A kind_len of zero is not a reserved value — an audit nobody wrote is also all
     * zeros — so a reader needs a field that is zero ONLY in the never-happened case. */
    uint64_t present;
} KayfabeDoorbellRefusal;

/* ★★★ E10e — a doorbell the SHELL served itself: one sentence naming what the CPU
 * copy-engine executor did (spans run, bytes moved, where the finishPayload landed).
 *
 * ⊘ Its own structure rather than a second KayfabeDoorbellRefusal.  The two carry the same
 * bytes and mean opposite things, and a header in which a serving is declared as a refusal
 * is a header that reads as a bug.  It carries no `kind` because there is exactly one way to
 * be served locally; a constant name would be a field that never varies. */
typedef struct KayfabeDoorbellServing {
    uint8_t text[KAYFABE_DOORBELL_REFUSAL_LEN];  /* NUL-PADDED, not NUL-terminated */
    uint64_t len;
    /* ⊘ Non-zero exactly when the shell served a doorbell itself — the validity flag, for
     * KayfabeDoorbellRefusal::present's reason. */
    uint64_t present;
} KayfabeDoorbellServing;

typedef struct KayfabeIsolateRefusal {
    uint8_t text[KAYFABE_ISOLATE_REFUSAL_LEN];  /* NUL-PADDED, not NUL-terminated */
    uint64_t len;
    uint64_t kind;
} KayfabeIsolateRefusal;

/* ★★★ THE CONTROL CENSUS — the two POSITIVE states the report could not express.
 *
 * The unserviced list says what NOTHING answered.  A refusal that ANSWERS (a non-zero
 * rpc_result, e.g. InitTablePolicy's refuse()) never reaches it — 0x20800301 was the
 * control named in the guest line that killed a boot while being absent from every list
 * the report printed — and a control that is SERVED is also absent, so "id absent" was
 * consistent with never-issued AND with served-fine and discriminated neither.  These rows
 * record seen-and-served and seen-and-refused positively, so absence finally means "never
 * seen".
 *
 * `served_len` / `arming_len` report the truth even when they exceed the arrays.  ⚠ So does
 * `unserviced_len` — but only since ABI 23; before that it was the sample's clamped length
 * and a saturated ledger was indistinguishable from a complete one.  KAYFABE_CTRL_NO_REPLY marks an arming no policy answered (the
 * FSM refused it by name) — deliberately not 0, which is NV_OK. */
/* ⊘ 32 -> 64.  Unlike unserviced_len, served_len WAS truthful past the array — it is a
 * counter kept beside the sample — but [measured 2026-08-09] that same boot reported
 * exactly 32 distinct served rows against a 32-slot array, so the very next control this
 * port served would have been counted and NOT shown. */
#define KAYFABE_SERVED_CONTROL_SLOTS 64u
#define KAYFABE_NOTIFIER_ARMING_SLOTS 16u
#define KAYFABE_CHANNEL_BIND_SLOTS 16u
#define KAYFABE_CTRL_NO_REPLY 0xFFFFFFFFu
/* The ce_index of a bind naming something that is NOT a copy engine, or whose params were
 * too short.  ⊘ Not 0 — 0 is CE0, and CE0 is one of the two indices this chip's captured
 * interrupt table publishes with vectorNonStall = INVALID. */
#define KAYFABE_BIND_NOT_A_COPY_ENGINE 0xFFFFFFFFu
#define KAYFABE_PROBE_ARM_SLOTS 8u

typedef struct KayfabeServedControl {
    uint32_t cmd;         /* the NV*_CTRL_CMD_* id */
    uint32_t rpc_result;  /* 0 = served; non-zero = served-but-REFUSED */
    uint64_t count;
} KayfabeServedControl;

/* One 0x20800301 arming, WITH the handles it arrived on.  The handles are the point: the
 * device's notify_actions is device-global while RM's already-armed rule is per-subdevice
 * (ogkm-580: subdevice_ctrl_event_kernel.c:126-131), so a second arming of one index on a
 * DIFFERENT subdevice must be visible as two rows with different `object` handles. */
typedef struct KayfabeNotifierArming {
    uint32_t client;      /* hClient from the control header */
    uint32_t object;      /* hObject — the subdevice armed on */
    uint32_t event;       /* notifier index, or KAYFABE_CTRL_NO_REPLY if undecodable */
    uint32_t action;      /* DISABLE=0 / SINGLE=1 / REPEAT=2, same marker */
    uint32_t rpc_result;  /* as answered, or KAYFABE_CTRL_NO_REPLY if nothing answered */
    uint32_t reserved;
    uint64_t count;
} KayfabeNotifierArming;

/* ★★★ ONE 0xa06f0104 CHANNEL BIND — the ONLY place the scrubber's chosen copy engine
 * becomes observable to this device.
 *
 * RmInitAdapter's global CeUtils scrubber picks its CE in ceutilsGetFirstAsyncCe — the
 * first CE that is not a GRCE and is in the engine table (ogkm-580: ce_utils.c:66-81) —
 * and kchannelBindToRunlist_IMPL RPCs it to us as `engineType`
 * (ogkm-580: kernel_channel.c:2762-2785).  On GA106 kceGetGrceMaskReg is halified to the
 * NV_ERR_NOT_SUPPORTED stub (ogkm-580: g_kernel_ce_nvoc.c:847-858), so the GRCE test walks
 * the partner list over THE DEVICE-INFO TABLE THIS PORT SERVES — which makes inferring the
 * answer from our own table circular.  It has to be read off the wire, and this row is it.
 *
 * Why it matters: the captured GA106_INTR_TABLE gives CE0 and CE1 vectorNonStall = INVALID
 * and CE2/CE3/CE4 a real vector, so which CE the guest bound is the difference between a
 * refusal grounded in "we deliver nothing" and one grounded in "the hardware we imitate
 * publishes no vector to raise".
 *
 * `engine_type` is raw NV2080_ENGINE_TYPE space and is NOT translated: above 0x12 that
 * space collides with RM_ENGINE_TYPE (raw 0x13 is NVDEC0 in one and COPY10 in the other). */
typedef struct KayfabeChannelBind {
    uint32_t client;      /* hClient from the control header */
    uint32_t object;      /* hObject — the channel being bound */
    uint32_t engine_type; /* NV2080_ENGINE_TYPE space, or KAYFABE_CTRL_NO_REPLY */
    uint32_t ce_index;    /* which CE that names, or KAYFABE_BIND_NOT_A_COPY_ENGINE */
    uint32_t rpc_result;  /* as answered, or KAYFABE_CTRL_NO_REPLY if nothing answered */
    uint32_t reserved;
    uint64_t count;
} KayfabeChannelBind;

/* ★★★ THE VA-SPACE PAGE-DIRECTORY PUBLICATION — 0x90f10106 / 0x20800a9f.
 *
 * MEASURED 2026-08-08 over traces/real_ga106/rpc_transcript_real_ga106.txt (a real
 * 580.159.04 driver on a real GA106), all 88 KAYFABE-RPC entries:
 * NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY occurs ZERO times, 0x90f10106 occurs FOUR times
 * and 0x20800a9f once.  SET_PAGE_DIRECTORY is the only control the port turns into a page
 * directory base, so these two ids are the ONLY boot-path source of one — and the port
 * decoded them, answered them NV_OK and discarded the value.
 *
 * `object` is the field that makes a row mean anything: the client arm is issued with
 * rmCtrlParams.hObject = hVASpace (ogkm-580: gpu_vaspace.c:5174-5177), so the RPC header —
 * not the params — is the only statement of WHICH address space these levels root.
 *
 * `levels[0]` is the ROOT.  _gvaspacePopulatePDEentries fills levels[] top-down from
 * pFmt->pRoot (gpu_vaspace.c:3974-4031) and the receiver consumes it bottom-up (:4492).
 * `aperture` is a real fork, not decoration: GMMU_APERTURE_VIDEO -> ADDR_FBMEM,
 * SYS_COH/SYS_NONCOH -> ADDR_SYSMEM, and the receiver asserts on anything else (:4503-4511).
 *
 * ⊘ `gvas_pub_undecodable` is a SEPARATE number, not an absent row: "the guest published
 * something we could not read" and "the guest published nothing" are different diagnoses
 * and only one of them is our defect.  Same argument as bar_pde_updates' refusal half. */
/* ★★★ 8 -> 32 at execution_plane_increments.md §16.6.  MEASURED 2026-08-09: six
 * consecutive boots each published 12 total / 11 DISTINCT VA spaces and this array showed
 * the first EIGHT — so (hClient 0xc1d0000a, hObject 0xcaf00005), the pair every one of
 * those boots names in its doorbell refusal, had its body printed in NONE of them.  §16.3
 * repaired the LOOKUP (it now reads a 256-row table) and left the REPORT at eight, which is
 * how the one row that decides the wall stayed invisible for three more boots. */
#define KAYFABE_GVAS_PUBLICATION_SLOTS 32u
#define KAYFABE_GVAS_MAX_LEVELS 6u

typedef struct KayfabePdeLevel {
    uint64_t phys_address;  /* GUEST-physical, in the guest's own frame of reference */
    uint64_t size;          /* bytes allocated for this level instance */
    uint32_t aperture;      /* GMMU_APERTURE_* */
    uint32_t page_shift;    /* NvU8 on the wire, widened here so the row needs no padding */
} KayfabePdeLevel;

typedef struct KayfabeGvasPublication {
    uint32_t cmd;           /* 0x90f10106 (client arm) or 0x20800a9f (global arm) */
    uint32_t client;        /* hClient from the RPC control header */
    uint32_t object;        /* ★ hObject — the VA SPACE.  See the block above. */
    uint32_t num_levels;    /* how many of levels[] are meaningful; 4 on GA106 */
    uint64_t page_size;     /* VA coverage of the level being reserved */
    uint64_t virt_addr_lo;  /* first GPU VA of the reserved range */
    uint64_t virt_addr_hi;  /* LAST GPU VA of the range, inclusive */
    uint32_t h_subdevice;   /* 0 on every occurrence measured; means "use subdevice_id" */
    uint32_t subdevice_id;
    uint64_t count;         /* how many times this exact row arrived */
    KayfabePdeLevel levels[KAYFABE_GVAS_MAX_LEVELS];
} KayfabeGvasPublication;

typedef struct KayfabeRegAudit {
    uint64_t reads;
    uint64_t writes;
    uint64_t boot_reg_reads;
    uint64_t ptimer_reads;
    /* #128 — writes to the free-running counter, REFUSED BY NAME (not dropped). */
    uint64_t ptimer_writes_refused;
    uint64_t rom_reads;
    uint64_t gsp_reads;
    uint64_t gsp_writes;
    uint64_t unclaimed_reads;
    uint64_t unclaimed_writes;
    /* ★★★ FRAMEBUFFER-WINDOW ACCESSES — DEVICE MEMORY, NOT REGISTERS.
     *
     * PRAMIN, the framebuffer aperture and the instance/BAR2 window are memory, and a page
     * table lives in memory.  The `unclaimed` counters above are honest about a defaulted
     * register value; these two are about something worse, so they are counted apart: a
     * dropped framebuffer write can be a dropped page-table entry, which does not fail
     * here — it fails much later as a mapping that is simply absent, at an address that
     * names nothing.
     *
     * MEASURED (2026-07-31, the committed C reference traces): the cold boot carries
     * 177856 instance-window + 33978 PRAMIN + 2 aperture writes, the matmul 214552 +
     * 33978 + 1511.  Before these fields every one of them was indistinguishable from an
     * unknown register offset. */
    uint64_t fb_window_reads;
    uint64_t fb_window_writes;
    /* ★★★ #146 — THE BAR0 MOVING WINDOW, SERVED.
     *
     * The two counters above now describe only the windows this port has NO ADDRESS MODEL
     * for — the GMMU-translated framebuffer aperture and instance window.  PRAMIN is
     * untranslated (the framebuffer address IS the window base plus the offset), so it has
     * a real address model and a real byte store, and it is counted apart.
     *
     * `fb_refusals` is the number to read.  It answers "did this boot drop a framebuffer
     * write?" — the question that used to be answerable only by kbusVerifyBar2 failing. */
    uint64_t fb_reads;
    uint64_t fb_writes;
    uint64_t fb_refusals;
    /* ★★★ #149 — THE TRANSLATED WINDOW, SERVED.
     *
     * The instance/BAR2 window is GMMU-translated: an access to it is a virtual address in
     * a page-table tree the guest built in framebuffer and whose ROOT ENTRY it published
     * over UPDATE_BAR_PDE.  `bar2_reads`/`bar2_writes` count accesses a page walk resolved;
     * `bar2_faults` counts the ones it refused BY NAME.
     *
     * Read the pair together.  kbusVerifyBar2's NV_ERR_MEMORY_ERROR cannot distinguish "the
     * walk never happened" from "the walk happened and landed on the wrong byte"; these can.
     *
     * `bar_pde_updates` packs (roots published << 32 | bodies refused).  The guest IGNORES
     * this command's status, so a refusal is invisible on its side and this is the only
     * place the arrival of a root is observable at all.  `bar2_root_entry` is the entry
     * itself — zero is a real value the guest publishes on teardown, which is why the count
     * and not the value is what says whether one arrived. */
    uint64_t bar2_reads;
    uint64_t bar2_writes;
    uint64_t bar2_faults;
    uint64_t bar_pde_updates;
    uint64_t bar2_root_entry;
    /* ★★★★ §16.18 — BAR1, THE FRAMEBUFFER APERTURE, TRANSLATED THE OTHER WAY ROUND.
     *
     * ⊘ READ `bar1_pde_base` FIRST — it is the PRECONDITION and it is carried, not implied.
     * BAR1's root does NOT arrive over UPDATE_BAR_PDE the way BAR2's does; MEASURED against
     * ogkm-580, NV_RM_RPC_UPDATE_BAR_PDE has exactly two call sites (kern_bus.c:880 and
     * kern_bus_gm107.c:2137) and BOTH pass NV_RPC_UPDATE_PDE_BAR_2.  What happens instead is
     * kbusPatchBar1Pdb_GSPCLIENT (kern_bus.c:755-807): the guest takes
     * GspStaticConfigInfo.bar1PdeBase — a number WE put in our own reply — and re-roots its
     * own page-table walker onto that framebuffer address.
     *
     * So `bar1_pde_base` is what we TOLD the guest, and a zero there means this port has no
     * framebuffer-aperture address model at all, which makes every other number in this
     * group a statement about nothing.  `bar1_root_published` is the counter-evidence: 1 iff
     * the guest ever sent an UPDATE_BAR_PDE naming BAR1 anyway.  It is expected to be 0, and
     * a 1 would refute the paragraph above rather than confirm it. */
    uint64_t bar1_reads;
    uint64_t bar1_writes;
    uint64_t bar1_faults;
    uint64_t bar1_pde_base;
    uint64_t bar1_root_published;
    uint64_t bar0_window_reads;
    uint64_t bar0_window_writes;
    uint64_t fb_resident_bytes;
    /* ★★★★ §16.13 — the residency CENSUS beside the total.  MEASURED (boot bar1_03a679f):
     * the report said "resident 368640 bytes" and the boot existed to answer "is the RING's
     * page one of them?", which a total cannot.  A sparse store returns ZEROS for a page
     * nobody ever wrote, so a byte census cannot tell "never written" from "written with
     * zeros"; residency can.
     * ⊘ fb_resident_valid is the PRECONDITION and it is carried, not implied.  A store that
     * backs no memory has no residency to report, and lo = hi = 0 would be a positive claim
     * about a device with no framebuffer port.  Zero here means "there was no store to ask",
     * and the printer says so in different words. */
    uint64_t fb_resident_valid;
    uint64_t fb_resident_lo;
    uint64_t fb_resident_hi;
    uint64_t fb_resident_pages;
    /* ★★★★ §16.16 — THE FIRST-WRITER CENSUS.  Indexed PRAMIN, BAR1, BAR2, EXEC,
     * UNATTRIBUTED (kayfabe_device::FbWriter::index).  How many resident pages each writer
     * was FIRST to touch — first and not last, because a page rewritten 6 900 times by a
     * later path would otherwise report that path and erase who CREATED it.
     * ⊘ READ THE `UNATTRIBUTED` SLOT FIRST.  MEASURED at tree e394b69: §16.15 built the
     * whole tagging mechanism and wired NONE of it — `write_tagged` had no caller anywhere
     * in the repo — so every write took the default and recorded UNATTRIBUTED.  A large
     * count in that slot is a fact about US ("a write path is not instrumented"), never a
     * finding about the guest.
     * ⊘ Precondition: fb_resident_valid, as for the extent above. */
    uint64_t fb_origin_by_writer[5];
    /* ★★★★ §16.16 — THE FORWARD SEARCH FOR THE RING, and it consults no page-table walk.
     * Every other instrument here takes the guest's declared ring VA, descends the guest's
     * page tables and reports where it lands — all of them sharing the premise that the
     * table being descended is the table the guest wrote the ring through.  A second
     * projection of one computation cannot audit the first.  This asks the CONVERSE: is
     * there a page ANYWHERE in our framebuffer whose bytes look like a GPFIFO ring?
     *   found nowhere        => the ring's bytes are not in our framebuffer at all
     *   found, NOT at the walk's leaf => we caught the write and are descending the WRONG
     *                          TABLE; the address plane is the defect, not the write path
     * `swept` is carried beside the resident total so "none found" can never be read as
     * "we looked at all of them" under truncation. */
    uint64_t fb_sweep_swept;
    uint64_t fb_sweep_ringlike;
    uint64_t fb_sweep_best;
    uint64_t fb_sweep_best_score;
    /* FbWriter::index PLUS ONE, so zero is "no origin recorded" and never PRAMIN. */
    uint64_t fb_sweep_best_writer_plus1;
    uint64_t faults;
    uint64_t ram_refusals;
    uint64_t irq_requests;
    /* ★★★ #151 — the CPU interrupt tree.  `cpu_intr_raises` is the one to read: the
     * driver's own loopback self-test triggers EXACTLY ONCE, so 1 is the healthy value.
     * `cpu_intr_masked` counts the triggers real silicon would have suppressed on the
     * enable bits, which this device deliberately does not gate on — see
     * kayfabe_device::cpuintr. */
    uint64_t cpu_intr_accesses;
    uint64_t cpu_intr_raises;
    uint64_t cpu_intr_masked;
    /* ★★★ §14.18 — THE COMPLETION NOTIFICATION.  A copy-engine submission this shell
     * served on the CPU ends with the bound engine's `vectorNonStall` latched into the
     * interrupt tree, because serving notifier index 35 is a promise to raise one when the
     * work completes (see kayfabe_abi::eventnotify).
     *
     * ⊘ READ `nonstall_unvectored` FIRST: it counts copies that really happened and were
     * never announced, which is the promise being broken quietly.  Its healthy value is 0.
     * `nonstall_masked` is the other half of a hang diagnosis — the message was delivered
     * but the guest's own LEAF_EN would hide the vector from its non-stall scan
     * (ogkm-580: intr_nonstall_tu102.c:344-346), and without this number that is
     * indistinguishable from never having raised. */
    uint64_t nonstall_raises;
    uint64_t nonstall_unvectored;
    uint64_t nonstall_masked;
    uint64_t commands;
    /* ★★★ THE LIST A BOOT IS WORTH.
     *
     * The emulated GSP's default answer to a command no policy models is a NAMED REFUSAL
     * (NV_ERR_NOT_SUPPORTED in the RPC envelope), never the request echoed back — an echo
     * hands the guest its own uninitialised stack and was MEASURED to page-fault the guest
     * kernel inside kbusInitBarsSize_KERNEL.
     *
     * That refusal is QUIET IN THE GUEST: the resource manager logs NV_ERR_NOT_SUPPORTED
     * at its INFO level, which a release module never prints.  So without these three
     * fields, "which controls has this port not built yet" is answerable only one guest
     * boot at a time.  Each entry is (function << 32) | cmd, with
     * KAYFABE_UNSERVICED_NO_CMD in the low half when the function was not a GSP_RM_CONTROL.
     * `unserviced_len` is the truth even when it exceeds the array. */
    uint64_t commands_unserviced;
    uint64_t unserviced_len;
    uint64_t unserviced[KAYFABE_UNSERVICED_SLOTS];
    /* ★★★ REFUSALS RAISED INSIDE THE OBJECT BRIDGE — AND WHY THEY NEED THEIR OWN FIELDS.
     *
     * The list above is "commands NO policy answered".  A bridge refusal DOES answer the
     * command — with a non-zero rpc_result — so it can never appear there, and before
     * these fields the port had no channel that said one had happened at all.
     *
     * MEASURED, boot `alloc1` at rev 2ced035 (docs/design/boot_measured_2026_08_01.md §6):
     * every GSP_RM_ALLOC was refused ParamsSizeExceedsPayload inside the bridge, and the
     * only evidence was that `fn 103` was ABSENT from a list of six.  Diagnosis by absence
     * is precisely what the unserviced ledger exists to abolish for the other half of the
     * chain; these fields finish the job. */
    uint64_t bridge_refusals;
    uint64_t bridge_refusal_len;
    KayfabeBridgeRefusal bridge_refusal[KAYFABE_BRIDGE_REFUSAL_SLOTS];
    /* ★★★ E1 — THE ISOLATE PLANE, AND WHY IT NEEDS ITS OWN FIELDS.
     *
     * `isolates_materialized` is the E0b number: since the spawn became LAZY, an isolate
     * exists because the GUEST caused an RM event, so 0 means the guest never got that far.
     * Before E0b it was 1 unconditionally, at device-realize time, 28 seconds before the
     * guest existed (MEASURED, rev e10a6bf, runs e0real2/e0real3).
     *
     * ⊘ It is NOT the instrument that attributes a spawn to the guest — the device writes
     * it, so it can say WHETHER and never WHY.  scripts/bench/e0_isolate_witness.sh is,
     * because it stamps host /proc sightings against boot_capture's own phase lines.
     *
     * `isolates_spawn_failed` is the E1 number: bench_rebuild_notes.md §5 row 7 recorded
     * that a FAILED real isolate was indistinguishable from a deliberately plane-less one
     * at the seam — both are "retired, no workers, checkout returns None".  They are two
     * different diagnoses and only one of them means the host is wrong. */
    uint64_t isolates_materialized;
    uint64_t isolates_live;
    uint64_t isolates_no_plane;
    uint64_t isolates_spawn_failed;
    KayfabeIsolateRefusal isolate_refusal;
    /* ★★★ E2 — THE USERMODE DOORBELL APERTURE.
     *
     * `doorbells` is the ARRIVAL count: guest MMIO writes that landed on
     * NV_VIRTUAL_FUNCTION_DOORBELL.  It is incremented before the core is consulted, so it
     * is a statement about the GUEST and nothing the core decides can reduce it, and
     * doorbells == doorbells_served + doorbells_refused always holds.  Neither of the two
     * can absorb the other, so "the transport works and the routing does not" — which is
     * exactly what E2 expects to read before E5 exists — is a state and not a silence.
     *
     * ⊘ A boot in which `doorbells` is ZERO is not evidence that the transport is broken.
     * At the current wall the guest driver never reaches kfifoUpdateUsermodeDoorbell: the
     * channel SCHEDULE before it fails (status 0x56), and the doorbell is rung from
     * channel_utils.c only after scheduling succeeds.  See
     * docs/design/execution_plane_increments.md §7.
     *
     * `doorbell_last_token_valid` exists because token 0 is a LEGAL work-submit token
     * (runlist 0, channel 0), so one field could not tell "rang channel 0" from "never
     * rang" — the same two-fields-for-one-fact argument fb_landed_valid already carries. */
    uint64_t doorbells;
    uint64_t doorbells_served;
    /* ★★★★ §16.62.3 — the SPLIT of `doorbells_served`, because that number was being read
     * as progress without saying WHOSE progress.  `_locally` is a copy THIS PROCESS ran and
     * whose end it witnessed; `_forwarded` is a host channel rung at an instant this device
     * was not standing at.  They are different events with different evidence, and
     * `_locally + _forwarded == doorbells_served` always. */
    uint64_t doorbells_served_locally;
    uint64_t doorbells_served_forwarded;
    /* ★★★★ §16.65 — THE PER-ENGINE DOORBELL CENSUS, bucketed in kayfabe_engine_kind_name()
     * order.
     *
     * ⊘ Why a whole array and not a headline: the three numbers above cannot tell
     * "EngineKind does not partition doorbell traffic" from "the engine refinement never
     * reached UVM's channels" — both refutations produce the SAME arrived/served/refused
     * triple, and the only other per-channel evidence in a boot log is the 16-line bounded
     * doorbell sample.  A bounded sample is not a census.
     *
     * ⊘ Fixed-width with every bucket printed, zeros included: an empty bucket is a
     * MEASUREMENT ("no NVENC channel rang"), and a sparse encoding would make it
     * indistinguishable from "we did not look".
     *
     * `sum(doorbells_by_engine) + doorbells_engine_unrouted == doorbells`, always;
     * `_unrouted` is a doorbell whose channel did not resolve, so no engine could be named.
     * ⊘ It is its own bucket and is never folded into "Other" — "Other" is an engine we
     * found and do not interpret, this is a channel we did not find. */
    uint64_t doorbells_by_engine[KAYFABE_ENGINE_KINDS];
    uint64_t doorbells_engine_unrouted;
    uint64_t doorbells_refused;
    uint64_t doorbell_last_token;
    uint64_t doorbell_last_token_valid;
    KayfabeDoorbellRefusal doorbell_refusal;
    /* ★★★ E10e — the LAST doorbell the shell's own CPU copy-engine executor served, and
     * what it did.  Last, where the refusal above is first: a refusal is a diagnosis and a
     * flood of rings must not push it out; a serving is PROGRESS, and the last one is how
     * far the guest got.  memmgrTestCeUtils issues a MemSet and then a MemCopy
     * (ogkm-580: mem_mgr.c:463, :467), so "which was the last to land" is the question. */
    KayfabeDoorbellServing doorbell_local_serving;

    /* ★★★ §8.2.2 — THE GPFIFO RING THE GUEST DECLARED.
     *
     * `kayfabe_arch::PushRange::gpa` is handed to the guest-RAM port with no walk, while a
     * GA10x GPFIFO entry names a GPU VIRTUAL address (ogkm-580: clc56f.h:270,272; the
     * driver path at this wall computes it as pbGpuVA + channelPbSize,
     * mem_utils_gm107.c:1232, where pbGpuVA is an NV04_MAP_MEMORY_DMA dmaOffset, :842).
     * Whether that mismatch is LIVE or merely LATENT turns on one number nobody had ever
     * looked at: the address the guest itself names for a ring, at this wall.  These four
     * fields are that number, carried out of a boot.
     *
     * ⊘ Recorded at TRANSLATION, so an alloc the graph then refused is still counted: the
     * question is what the GUEST named, not what we accepted.
     *
     * `gpfifo_ring_nonzero` is the validity flag for the two below, and it is not
     * redundant — gpFifoOffset = 0 is a declaration the driver makes ON PURPOSE for its
     * golden-context channel (ogkm-580: kernel_graphics.c:2420-2424), so a single field
     * could not tell "declared address zero" from "declared nothing".  Same argument as
     * doorbell_last_token_valid above. */
    uint64_t gpfifo_ring_declarations;
    uint64_t gpfifo_ring_nonzero;
    uint64_t gpfifo_ring_va;
    uint64_t gpfifo_ring_entries;

    /* ★★★ THE CONTROL CENSUS — see the block above KayfabeServedControl.  Together with
     * `unserviced` and `bridge_refusal` these cover the command stream's three states:
     * seen-and-served, seen-and-refused, and (by positive elimination) never seen. */
    uint64_t served_total;
    uint64_t served_len;
    KayfabeServedControl served[KAYFABE_SERVED_CONTROL_SLOTS];
    uint64_t arming_total;
    uint64_t arming_len;
    KayfabeNotifierArming armings[KAYFABE_NOTIFIER_ARMING_SLOTS];

    /* ★★★ THE CHANNEL-BIND CENSUS — see the block above KayfabeChannelBind.  `bind_len` is
     * the number of DISTINCT rows and is the truth even when it exceeds the array. */
    uint64_t bind_total;
    uint64_t bind_len;
    KayfabeChannelBind binds[KAYFABE_CHANNEL_BIND_SLOTS];

    /* ★★★★ §16.40 — THE FIRST REFUSED GPU_PROMOTE_CTX, WITH THE ADDRESS PLANE'S STATE AS IT
     * STOOD AT THAT INSTANT.  NUL-PADDED, not NUL-terminated; print with an explicit
     * precision from `promote_diag_len`.
     *
     * ⊘ `promote_diag_len == 0` is a FINDING, not a blank: it means no promotion was ever
     * refused.  Read beside the `0x2080012b` rows in the served-control census it separates
     * "every promotion succeeded" from "none arrived".  It never means the instrument was
     * off — the sentence is latched by the bridge at the moment it refuses.
     *
     * ★★★ WHY THIS FIELD EXISTS.  The per-channel VA-space census it carries has existed
     * since §15 and was reachable ONLY from inside a doorbell-refusal sentence.  MEASURED
     * 2026-08-09: the string `census[` appears in exactly TWO of the seventeen boot logs in
     * traces/guest_boots/, and in none since doorbells began to be SERVED — s35 reports
     * `doorbells: 124 arrived, 124 served, 0 REFUSED`, so the refusal that carried the
     * census never happened.  A diagnostic for the ADDRESS plane was gated on the EXECUTION
     * plane failing, and fixing the second silenced the first with no line in the report to
     * say so.  Three rungs then recorded "which VA space the channel names is unread". */
    KayfabePromoteDiag promote_diag[KAYFABE_PROMOTE_DIAG_SLOTS];
    /* DISTINCT kinds latched — the truth even past the array.  0 = none refused. */
    uint64_t promote_diag_len;

    /* ★★★ THE VA-SPACE PAGE-DIRECTORY PUBLICATIONS — see the block above
     * KayfabeGvasPublication.  `gvas_pub_total` counts every publication that decoded,
     * `gvas_pub_len` is the number of DISTINCT rows and is the truth even when it exceeds
     * the array, and `gvas_pub_undecodable` counts publications that arrived and did not
     * decode.  A boot with all three at zero is a boot in which the guest never published
     * a page directory at all — which is a diagnosis, not a silence. */
    uint64_t gvas_pub_total;
    uint64_t gvas_pub_len;
    uint64_t gvas_pub_undecodable;
    /* ★★★★ IS THE ROOT TABLE STILL COMPLETE?  The healthy value is ZERO.
     *
     * The rows above are a bounded REPORT.  The lookup that decides whether a guest channel
     * can address anything is a separate, much larger table
     * (`kayfabe_device::gvaspub::GVAS_ROOT_TABLE_MAX`), and this counts publications it had
     * to refuse.
     *
     * ⊘ It exists because of what its absence cost.  `[measured 2026-08-09, boot
     * uvm1_b731e3c]` the resolver looked VA spaces up in the EIGHT-ROW report sample during
     * a boot that published ELEVEN distinct, so three address spaces were refused with
     * "the guest published no page-directory root" — a false statement about the guest.
     * A non-zero value here invalidates every such refusal in the same boot. */
    uint64_t gvas_pub_roots_refused;
    /* ★★★ THE SEAT THAT CARRIES A PUBLICATION INTO THE OBJECT MODEL (§14.23), counted by a
     * DIFFERENT link from the three above.  `gvas_pub_total` is the recorder's (decode +
     * log); `gvas_pub_seen` is the observer's (decode + declare), and `gvas_pub_applied` is
     * how many the object model ACCEPTED — i.e. how many VA spaces now carry the guest's
     * own page-directory base.
     *
     * ⊘ Two counts of one event on purpose.  Until 2026-08-08 the port RECORDED this
     * control and answered NV_OK without forwarding it, so `gvas_pub_total` was 5 while the
     * object model held nothing; a single number could not have said that.  A boot with
     * `gvas_pub_total` non-zero and `gvas_pub_seen` zero is a front seat that was never
     * filled.  `gvas_pub_unexpected` is unreachable by construction and printed anyway. */
    uint64_t gvas_pub_seen;
    uint64_t gvas_pub_applied;
    uint64_t gvas_pub_unexpected;
    KayfabeGvasPublication gvas_pub[KAYFABE_GVAS_PUBLICATION_SLOTS];
    /* ★ THE PROBE SET THIS BOOT RAN WITH — from the `probe-arm-notifier` device property,
     * recorded by the plane's census at construction from the same value the event-plane
     * arm consults.  0 entries in every shipping boot.  Printed so a boot's own report
     * proves its probe set: the predecessor env var ran three boots probe-off while
     * looking armed from the launching shell.  Never clipped: the parser refuses more
     * than the array holds, so probe_arm_len <= KAYFABE_PROBE_ARM_SLOTS by construction. */
    uint64_t probe_arm_len;
    uint32_t probe_arm[KAYFABE_PROBE_ARM_SLOTS];

    /* ★★★ §14.41 -- replayable fault buffers the guest registered and this port ANSWERED
     * NV_OK to (0x20800a9b).  Answering it is what lets cuInit past faultbufConstruct_IMPL,
     * and it buys REGISTRATION ONLY: nothing in this build raises a replayable fault or
     * advances MMU_FAULT_BUFFER_PUT(1).
     *
     * The count is the printer's TRIGGER, not the point.  A served row in the control census
     * reads as "handled", which is exactly the too-capable-mock reading this project keeps
     * being bitten by -- so when this is non-zero the printer emits the delivery-unbuilt
     * sentence beside it.  Every boot that serves the control also reports what the control
     * did not buy.
     *
     * ⚠ A value > 1 is a FINDING: the physical receiver returns NV_ERR_NOT_SUPPORTED on a
     * second registration while one is live (ogkm-580: kern_gmmu.c:3117) and this port does
     * not model that, deliberately -- its 0x20800a9c partner is unserved, so the state could
     * only ever latch shut.  The repeats are counted so the decision is made on a
     * measurement. */
    uint64_t fault_buffers_registered;
    /* faultBufferSize of the FIRST registration, in bytes, or 0 if none decoded. */
    uint64_t fault_buffer_size;
    /* PTE entries the guest actually filled for that first registration.  ★ 49 on a stock
     * GA106 -- 0x20800a59's advertised replayableFaultBufferSize (0x31000) / RM_PAGE_SIZE.
     * Anything else on a stock boot means the two controls disagree. */
    uint64_t fault_buffer_pages;
    /* Registrations whose params did NOT decode.  ⊘ Its own counter rather than a silence:
     * "the guest never asked" and "the guest asked in a shape we could not read" are
     * different findings, and the second means this port's layout is wrong. */
    uint64_t fault_buffers_malformed;

    /* ★★★ §14.41 rung 2 -- CLIENT SHADOW fault buffers the guest registered (0x20800a9d).
     *
     * ⊘ Counted SEPARATELY from fault_buffers_registered, and the separation is the point.
     * The two controls carry different promises: answering 0x20800a9b says a register WE
     * serve will keep reading empty; answering this one says WE will write fault packets
     * into pages of the guest's own sysmem (ogkm-580: kern_gmmu.c:1589-1593, "GSP will be
     * writing the fault packets to these buffers").  On a GSP client the guest has NO other
     * route to a non-replayable fault -- the CPU driver never reads the hardware buffer
     * (unix_intr.c:933-938) and the interrupt services are registered only when
     * !IS_GSP_CLIENT (kern_gmmu.c:2267-2288).  One number could not say which promise a boot
     * took on, and the printer emits a different sentence for each. */
    uint64_t shadow_fault_buffers_registered;
    /* shadowFaultBufferSize of the FIRST shadow registration.  ★ 0x120c20 on a stock GA106 --
     * 0x20800a59's own advertised nonReplayableFaultBufferSize.  Anything else means the two
     * controls disagree about a buffer the guest has already allocated. */
    uint64_t shadow_fault_buffer_size;
    /* Pages the guest filled: align_up(size)/4096 + align_up(metadataSize)/4096
     * (ogkm-580: kern_gmmu.c:1601).  289 for the stock size. */
    uint64_t shadow_fault_buffer_pages;
    /* shadowFaultBufferType, RAW.  0 = non-replayable, the only value reachable with
     * Confidential Compute off; 1 = replayable shadow, which NEEDS CC
     * (ogkm-580: mmu_fault_buffer_ctrl.c:148).  ⚠ Anything but 0 is a FINDING this port
     * deliberately does not refuse on -- refusing would model a path no measurement has
     * reached. */
    uint64_t shadow_fault_buffer_type;
    /* Shadow registrations whose params did NOT decode. */
    uint64_t shadow_fault_buffers_malformed;

    /* ★★★ ACCESS-COUNTER notification buffers the guest registered (0x20800a1d) -- the third
     * buffer, and the sharpest: it is the only one whose SIZE this port also invents
     * (ga10x's ACCESS_COUNTER_NOTIFY_BUFFER_ENTRIES_ADVERTISED, an admitted fiction serving
     * BAR0 0xB83110).  The printer says both halves.
     *
     * ⚠ 0 here AFTER a cuInit is a FINDING, not a quiet success: the control is only
     * reachable once 0xB83110 stops reading zero, so its absence from every ledger before
     * §14.41 was evidence of nothing. */
    uint64_t access_cntr_buffers_registered;
    /* bufferSize of the first, in bytes.  ★ 8192 = 256 advertised entries x 32. */
    uint64_t access_cntr_buffer_size;
    /* Pages the guest filled -- 2 for the advertised size. */
    uint64_t access_cntr_buffer_pages;
    /* Access-counter registrations whose params did NOT decode. */
    uint64_t access_cntr_buffers_malformed;
    /* ★★★★ §16.30 -- NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY (0x00801813) ACCEPTED,
     * including re-installations.  ⚠ 0 here is a FINDING: the two boots that named this
     * control REFUSED it, and RM's rollback is what fires the one dmesg line unique to
     * cuInit's window.  A boot leaving this at 0 did not test the rung. */
    uint64_t set_page_dir_total;
    /* Arrived and were REFUSED -- serialized params, or a declared size that is not
     * sizeof.  Non-zero invalidates the record below. */
    uint64_t set_page_dir_refused;
    /* ★★★ Whether the record below means anything.  ⊘⊘ READ THIS FIRST.  hVASpace == 0 is
     * a REAL handle value naming the client/device pair's implicit VA space, so a reported
     * 0 with no valid bit beside it cannot be told from "no SET ever arrived".  Every
     * field below is ambiguous at zero. */
    uint64_t set_page_dir_valid;
    /* hClient from the RPC control header. */
    uint64_t set_page_dir_client;
    /* hObject from that header -- hDevice, NOT the VA space.  ⚠ The opposite convention
     * from 0x90f10106, whose header hObject IS the VA space. */
    uint64_t set_page_dir_object;
    /* ★★★ hVASpace from the PARAMS -- reported exactly as it arrived, interpreted by
     * nobody.  Whether it is 0 (the Device's implicit VAS) or a real handle (a user VAS,
     * which is what UVM allocates) is the open question §16.30 exists to answer. */
    uint64_t set_page_dir_h_vaspace;
    /* physAddress -- guest-physical, in the aperture named by flags. */
    uint64_t set_page_dir_phys;
    /* numEntries -- decides RM's next three checks after we answer NV_OK. */
    uint64_t set_page_dir_num_entries;
    /* flags, raw -- aperture in bits 1:0, plus ALL_CHANNELS / EXTEND_VASPACE /
     * IGNORE_CHANNEL_BUSY. */
    uint64_t set_page_dir_flags;
} KayfabeRegAudit;

/* The identity a chip claims.  `device_id` of 0 selects the chip table's default row.
 * Takes no handle: it is a pure function of the table, and the device needs the answer at
 * class-init/realize time, before anything else exists. */
int32_t kayfabe_shim_chip_identity(uint16_t device_id, KayfabeChipIdentity *out,
                                   const uint8_t **out_msg, uint64_t *out_msg_len);

/* Create the register plane for a chip.  `device_id` of 0 selects the default row.
 *
 * `probe_arm`/`probe_arm_len` carry the `probe-arm-notifier` device property: a
 * comma-separated decimal list of notifier indices to PROBE-arm (reachability
 * instrumentation, never a shipping path).  Pass (NULL, 0) or an empty string for the
 * shipping configuration.  ⊘ Junk in the string refuses the device BY NAME rather than
 * booting probe-off — the predecessor env var silently did the latter, three boots in a
 * row.  The set in effect comes back in KayfabeRegAudit.probe_arm. */
int32_t kayfabe_shim_regs_create(uint16_t device_id,
                                 const uint8_t *probe_arm, uint64_t probe_arm_len,
                                 void **out_handle,
                                 const uint8_t **out_msg, uint64_t *out_msg_len);

void    kayfabe_shim_regs_destroy(void *handle);

/* ★★★ STAGE Q5.  Join the two planes: give the register plane the realized memory plane's
 * guest RAM, so the emulated GSP can follow the guest's own pointers.
 *
 * Call it once the memory plane has realized, which is LATER than the register plane's
 * creation — a memory plane needs a programmed base-address register and a register plane
 * does not.  Until then, and again after _detach_ram, every guest-memory access the
 * emulated GSP attempts is refused BY NAME rather than answered with zeros.
 *
 * Returns KAYFABE_E_MALFORMED if either handle is empty (the shim called in the wrong
 * order).  The symptom of the missing call is a device that boots normally and then refuses
 * one specific register write thousands of accesses in, so it is worth a code. */
int32_t kayfabe_shim_regs_attach_ram(void *regs, void *shim);

/* Put the register plane back to refusing every guest-memory access, by name.
 *
 * ★ Call this BEFORE kayfabe_shim_unrealize.  The register surface keeps answering across a
 * memory-plane teardown by design, so the port that reaches INTO the memory plane has to be
 * withdrawn explicitly — a lifetime the C side cannot express any other way. */
void    kayfabe_shim_regs_detach_ram(void *regs);

/* ★★ The archive's own name for the REGISTER aperture — `BUS_BAR_0`.
 *
 * The `bar` argument below is RM's LOGICAL base-address-register index, which is dense
 * (0..3) and is NOT the PCI slot number (0, 1, 3, 5 on this device, because two of the
 * windows are 64-bit).  nvkvm_regions is the only translation between the two; these two
 * names exist so the call sites read as a choice rather than as a magic number. */
#define KAYFABE_BUS_BAR_REGS 0u
/* ★★★ #149 — `BUS_BAR_2`, the instance window.  A GMMU-TRANSLATED aperture: the archive
 * walks the guest's own page tables to turn an offset in it into a framebuffer address,
 * rooted at the entry the guest published over UPDATE_BAR_PDE. */
#define KAYFABE_BUS_BAR_INST 2u
/* ★★★★ §16.18 — `BUS_BAR_1`, the framebuffer aperture.  Also GMMU-translated, but rooted
 * the OTHER WAY ROUND: not at an entry the guest published to us, at the page-directory
 * ADDRESS we published to the guest in GspStaticConfigInfo.bar1PdeBase.  See
 * KayfabeRegAudit::bar1_pde_base for why that direction is the whole story. */
#define KAYFABE_BUS_BAR_FB 1u

/* ★★ THE HOT PATH.  One trapped register access, one value.  `size` is the access width in
 * bytes and the answer is masked to it.  An empty handle reads zero — a device whose plane
 * failed to build must not crash a guest, and it has already said so loudly at realize. */
uint64_t kayfabe_shim_regs_read(void *handle, uint32_t bar, uint64_t off, uint32_t size);

/* One trapped register write.  `out` may be NULL if the caller does not care. */
void     kayfabe_shim_regs_write(void *handle, uint32_t bar, uint64_t off, uint32_t size,
                                 uint64_t val, KayfabeRegWrite *out);

/* Power-on reset: rebuild the emulated GSP's state machine.  The C artifact could only do
 * this by restarting the whole hypervisor process, which is the bench's fresh-boot tax. */
void     kayfabe_shim_regs_reset(void *handle);

int32_t  kayfabe_shim_regs_audit(void *handle, KayfabeRegAudit *out);

#endif /* KAYFABE_SHIM_H */
