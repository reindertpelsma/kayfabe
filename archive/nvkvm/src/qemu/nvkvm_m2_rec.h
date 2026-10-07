/*
 * nvkvm_m2_rec.h — the §6 replay-trace recorder (task #90).
 *
 * ONE interleaved, totally-ordered binary stream of everything the emulated GPU
 * observed or produced, so that a second implementation (the Rust `kayfabe`
 * rewrite) can be replayed against it and DIFFED.  The format and the rules are
 * specified in the rewrite's `docs/design/mode2_gsp_port_plan.md` §6/§6.3 and
 * `docs/design/c_rust_trace_differential.md`; this header carries only what a
 * reader of the C needs.
 *
 * ── The four rules this sink exists to obey ───────────────────────────────
 *
 * R1  ONE COUNTER, ONE STREAM.  Every emitted record carries a dense, strictly
 *     increasing `seq`.  This is a genuine total order and needs no lock: the
 *     device is single-threaded (no qemu_thread_create/qemu_bh_new/timer_new in
 *     nvkvm_gpu_emul.c, all four MemoryRegionOps keep `global_locking = true`,
 *     and the isolate reader thread never touches NvkvmGpuEmul).
 *
 * R2  NEVER SAMPLE, NEVER CAP.  The consumer's differential compares stream
 *     POSITIONS and is sequence-number-blind, so dropping every Nth record
 *     shifts every later index and manufactures a divergence at the first drop.
 *     There is therefore no counter cap anywhere in this file.  The only
 *     legitimate volume control is a DECLARED FILTER (`m2recmask`) that is
 *     written into the file header so the other recorder can apply the same one.
 *     A filtered-out event does NOT consume a sequence number — the stream stays
 *     dense under any mask.
 *
 * R3  RAW BYTES, NO FORMATTING ON THE HOT PATH.  Not qemu_log(): its formatting
 *     cost sits inside the traced region and it cannot carry DMA payloads.  One
 *     fd, one staging buffer, write(2) on overflow.
 *
 * R4  THE ARTEFACT CARRIES ITS OWN PROVENANCE.  The header holds the exact
 *     `m2*` property vector, the mask, and a free-text block (guest kernel
 *     vermagic, host driver version, nvidia-smi summary, hermeticity note).  An
 *     oracle whose provenance is not in the artefact stops being an oracle the
 *     moment the bench dies.
 *
 * ── File layout ──────────────────────────────────────────────────────────
 *
 *   [ NvkvmRecHdr ][ provenance text, hdr_len - sizeof(hdr) bytes ][ records ]
 *
 * Each record is a fixed 32-byte NvkvmRecEnt followed by `len` payload bytes
 * padded up to an 8-byte multiple, so every record starts 8-byte aligned.
 * All integers are little-endian (the only host this ever runs on).
 *
 * The record count / byte count / error count in the header are patched by
 * pwrite() at close.  If QEMU is killed they stay 0 — a reader must then scan
 * to EOF.  That is deliberate: a killed QEMU still leaves a usable DENSE PREFIX,
 * which is the only kind of truncation that is not fatal.
 */
#ifndef NVKVM_M2_REC_H
#define NVKVM_M2_REC_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#define NVKVM_REC_MAGIC   0x4352434552564B4EULL   /* "NKVRECRC" LE */
#define NVKVM_REC_VERSION 1u

/* ── record kinds ────────────────────────────────────────────────────────
 * 1..6 map 1:1 onto the rewrite's wire-plane `TraceEvent` variants.
 * 7 has NO counterpart there; see NVKVM_REC_OVERLAY below. */
enum {
    NVKVM_REC_MMIO_RD   = 1,  /* a=off  b=value SERVED  bar,width set        */
    NVKVM_REC_MMIO_WR   = 2,  /* a=off  b=value WRITTEN bar,width set        */
    NVKVM_REC_GUEST_RD  = 3,  /* a=gpa  len=n  tail = the bytes RETURNED     */
    NVKVM_REC_GUEST_WR  = 4,  /* a=gpa  len=n  tail = the bytes WRITTEN      */
    NVKVM_REC_IRQ       = 5,  /* a=0 -> MSI-X, b=vector; a=1 -> INTx, b=level*/
    NVKVM_REC_CLOCK     = 6,  /* a=ns on QEMU_CLOCK_VIRTUAL                  */
    /* ── C-only, no `TraceEvent` counterpart ──────────────────────────────
     * The m2romregs rom-device overlay serves BAR0 page 0x110000 out of RAM, so
     * the guest's reads of IRQSTAT/MAILBOX0/CPUCTL/DMATRFCMD NEVER TRAP and can
     * never appear as MMIO_RD.  §6.2 asks for a snapshot of the overlay page
     * instead, emitted whenever the page is re-synced.  A decoder should either
     * drop these or expand them into the MmioReads they stand in for; it must
     * not feed them to a positional differential against a recorder that has no
     * overlay.  Moot when m2romregs=off (the capture-1 configuration). */
    NVKVM_REC_OVERLAY   = 7,  /* a=BAR0 off (0x110000) len=4096 tail=page     */
};

/* ── the declared filter (m2recmask) ─────────────────────────────────────
 * Written into the header verbatim.  Bits 0-7 select record KINDS; bits 8-15
 * further restrict the two MMIO kinds by region.  An MMIO record must pass BOTH
 * its kind bit and its region bit. */
#define NVKVM_REC_M_MMIO_RD   (1ull << 0)
#define NVKVM_REC_M_MMIO_WR   (1ull << 1)
#define NVKVM_REC_M_GUEST_RD  (1ull << 2)
#define NVKVM_REC_M_GUEST_WR  (1ull << 3)
#define NVKVM_REC_M_IRQ       (1ull << 4)
#define NVKVM_REC_M_CLOCK     (1ull << 5)
#define NVKVM_REC_M_OVERLAY   (1ull << 6)

#define NVKVM_REC_M_BAR0      (1ull << 8)   /* BAR0 regs, minus the three below */
#define NVKVM_REC_M_BAR1      (1ull << 9)   /* PCI BAR1 — the FB aperture       */
#define NVKVM_REC_M_BAR2      (1ull << 10)  /* PCI BAR3 — RM "BAR2" GMMU window */
#define NVKVM_REC_M_PROM      (1ull << 11)  /* the ~1 MiB streamed VBIOS window */
#define NVKVM_REC_M_PRAMIN    (1ull << 12)  /* the BAR0 PRAMIN FB window        */
#define NVKVM_REC_M_PTIMER    (1ull << 13)  /* PTIMER_TIME_0/1 (poll storm)     */

#define NVKVM_REC_M_ALL       (~0ull)

/* ── the property vector (hdr.props) ─────────────────────────────────────
 * Exactly which `m2*` knobs were on for this run.  Recorded because several of
 * them change what the device DOES, not merely what it logs. */
#define NVKVM_REC_P_TRACE     (1ull << 0)
#define NVKVM_REC_P_M2FWD     (1ull << 1)
#define NVKVM_REC_P_M2EXEC    (1ull << 2)
#define NVKVM_REC_P_M2HOSTSEM (1ull << 3)
#define NVKVM_REC_P_M2CEFWD   (1ull << 4)
#define NVKVM_REC_P_M2CEXEC   (1ull << 5)
#define NVKVM_REC_P_M2OPAQUE  (1ull << 6)
#define NVKVM_REC_P_M2TRACE   (1ull << 7)
#define NVKVM_REC_P_M2ROMREGS (1ull << 8)
/* Derived, not a property: set when the run is NOT hermetic — i.e. the host GPU
 * could DMA into guest RAM behind this recorder (m2fwd and/or m2exec on).  A
 * trace with this bit set cannot be closed over by a replay; its value is the
 * decision planes, not replayability. */
#define NVKVM_REC_P_NONHERMETIC (1ull << 32)

typedef struct NvkvmRecHdr {
    uint64_t magic;        /* NVKVM_REC_MAGIC                                */
    uint32_t version;      /* NVKVM_REC_VERSION                              */
    uint32_t hdr_len;      /* bytes from file start to the first record      */
    uint32_t rec_size;     /* sizeof(NvkvmRecEnt) == 32                      */
    uint32_t reserved0;
    uint64_t props;        /* NVKVM_REC_P_*                                  */
    uint64_t mask;         /* NVKVM_REC_M_* actually applied                 */
    uint64_t n_records;    /* patched at close; 0 => scan to EOF             */
    uint64_t n_bytes;      /* patched at close                               */
    uint64_t n_errors;     /* short/failed write(2)s; MUST be 0              */
    uint64_t t0_ns;        /* QEMU_CLOCK_VIRTUAL at open                     */
    uint64_t reserved1[3];
    /* followed by hdr_len - sizeof(NvkvmRecHdr) bytes of NUL-padded free text */
} NvkvmRecHdr;

typedef struct NvkvmRecEnt {
    uint64_t seq;
    uint8_t  kind;
    uint8_t  width;        /* MMIO access size in bytes; 0 otherwise         */
    uint8_t  bar;          /* PCI BAR index for MMIO; 0xFF otherwise         */
    uint8_t  pad0;
    uint32_t len;          /* payload bytes following this record            */
    uint64_t a;
    uint64_t b;
} NvkvmRecEnt;

/* Open the sink.  `path` is the output file; `props`/`mask` go in the header;
 * `prov` is free-text provenance (may be NULL).  Returns true on success.
 * Idempotent: a second call while open is a no-op returning true. */
bool nvkvm_rec_open(const char *path, uint64_t props, uint64_t mask,
                    const char *prov, uint64_t t0_ns);

/* Flush, patch the header counters, close.  Safe to call more than once and
 * safe from an exit notifier. */
void nvkvm_rec_close(void);

/* True when a sink is open.  A load of a file-static bool — this is the guard
 * every call site uses, so the disabled cost is one predictable branch. */
bool nvkvm_rec_on(void);

/* The declared filter, as applied.  Call sites that would have to do real work
 * to build a payload should test this first. */
uint64_t nvkvm_rec_mask(void);

/* Append one record.  `payload`/`len` may be NULL/0.  Does nothing (and does
 * NOT consume a sequence number) when the sink is closed or when `kindbit` is
 * masked out. */
void nvkvm_rec_emit(uint64_t kindbit, uint8_t kind, uint8_t bar, uint8_t width,
                    uint64_t a, uint64_t b, const void *payload, uint32_t len);

/* How many records have been emitted so far (for the shutdown summary). */
uint64_t nvkvm_rec_count(void);

#endif /* NVKVM_M2_REC_H */
