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
#define KAYFABE_SHIM_ABI 5u

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
    uint32_t       transitions;
    uint32_t       commands;
    int32_t        claimed;            /* the register model owns this offset */
    int32_t        raise_status_irq;   /* the status queue wants announcing */
} KayfabeRegWrite;

/* Register-plane counters.  u64-only and address-free, like KayfabeAudit.
 *
 * ★ `unclaimed_reads` is the honest one: it counts every register this device answered
 * with a DEFAULTED ZERO because no model owns the offset.  It is not an error today (the
 * C artifact does the same, and refusing would mean the device could not boot until every
 * register in a 16 MiB aperture had a model) — it is the number that says how much of a
 * boot rests on that. */

/* How many distinct unserviced commands KayfabeRegAudit carries, and the low half of a
 * packed entry that names no control. */
#define KAYFABE_UNSERVICED_SLOTS 32u
#define KAYFABE_UNSERVICED_NO_CMD 0xFFFFFFFFu

typedef struct KayfabeRegAudit {
    uint64_t reads;
    uint64_t writes;
    uint64_t boot_reg_reads;
    uint64_t ptimer_reads;
    uint64_t rom_reads;
    uint64_t gsp_reads;
    uint64_t gsp_writes;
    uint64_t unclaimed_reads;
    uint64_t unclaimed_writes;
    uint64_t faults;
    uint64_t ram_refusals;
    uint64_t irq_requests;
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
} KayfabeRegAudit;

/* The identity a chip claims.  `device_id` of 0 selects the chip table's default row.
 * Takes no handle: it is a pure function of the table, and the device needs the answer at
 * class-init/realize time, before anything else exists. */
int32_t kayfabe_shim_chip_identity(uint16_t device_id, KayfabeChipIdentity *out,
                                   const uint8_t **out_msg, uint64_t *out_msg_len);

/* Create the register plane for a chip.  `device_id` of 0 selects the default row. */
int32_t kayfabe_shim_regs_create(uint16_t device_id, void **out_handle,
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
