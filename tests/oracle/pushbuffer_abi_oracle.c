/*
 * pushbuffer_abi_oracle — a TEST-ONLY differential oracle for increment **E4**, built out
 * of NVIDIA's OWN field definitions, NVIDIA's OWN bit-packing macros, and NVIDIA's OWN
 * USERD-size HAL.
 *
 * ============================================================================
 * WHY THIS EXISTS
 * ============================================================================
 *
 * `kayfabe_chips::Ga10xUserd` and `kayfabe_chips::Ga10xPushbuffer` replace the two
 * refusing stubs `UnbuiltUserd` / `UnbuiltPushbuffer`. Between them they decide:
 *
 *   - how large a channel's USERD is and where its two cursors sit — i.e. how much guest
 *     memory a USERD mapping covers;
 *   - which bytes of a GPFIFO ring are an address and which are a length;
 *   - how many argument words follow a pushbuffer header, which is what keeps a method
 *     parser SYNCHRONISED with the guest's own stream.
 *
 * Every one of those is a bit position, and a bit position got wrong does not fail: it
 * produces a plausible number. `execution_plane_increments.md`'s E4 row states the control
 * as *"garbage bytes must FAULT, not decode to a plausible method"* — and the mirror image
 * of that is this file: real bytes must decode to what the guest actually wrote.
 *
 * ⊘ **A round trip against our own encoder cannot settle any of it.**
 * `kayfabe_abi::submit::gp_entry` and `..::gp_entry_decode` are both ours; two functions
 * written from the same wrong belief agree with each other perfectly. That is the exact
 * shape `never_let_a_test_use_the_thing_under_test_as_its_own_observer` names, and the one
 * that let a planted mutation survive `MockArch::token_for` on 2026-08-01. So the
 * authority here is NVIDIA's header, compiled.
 *
 * ============================================================================
 * WHAT IS COMPILED, AND WHAT IS OURS
 * ============================================================================
 *
 * The only NVIDIA code here is:
 *
 *   1. `class/clc56f.h` (AMPERE_CHANNEL_GPFIFO_A) and `class/clc7b5.h`
 *      (AMPERE_DMA_COPY_B) — the class headers, unmodified, included by path. Every
 *      `GP_ENTRY*`, `DMA_*`, `SEM_*`, `MEM_OP_*` and `LAUNCH_DMA` field below is theirs.
 *   2. `nvmisc.h`'s `DRF_NUM` / `DRF_DEF` / `DRF_VAL` — the driver's own bit-packing.
 *      NO SHIFT AND NO MASK IS WRITTEN IN THIS FILE.
 *   3. `SF_OFFSET` / `SF_SHIFT` / `SF_MASK` — sliced byte-for-byte out of
 *      `generated/g_gpu_access_nvoc.h` by `tests/build.rs`. These turn a `dev_ram.h`
 *      field extent such as `NV_RAMUSERD_GP_GET` into a **byte offset**, which is the
 *      thing `UserdModel::gp_get_offset` answers.
 *   4. `kfifoGetUserdSizeAlign_<HAL>` — sliced byte-for-byte out of the driver file the
 *      driver's OWN dispatch table binds for GA106 (`tests/build.rs` parses
 *      `g_kernel_fifo_nvoc.c` for it), together with that file's OWN `published/…`
 *      includes. ★★ This one is not decoration: `kfifoGetUserdSizeAlign` is halified two
 *      ways, GA106 takes the fallback arm, and the fallback is **Maxwell's** — so an
 *      Ampere channel's USERD is sized out of `published/maxwell/gm107/dev_ram.h` and
 *      `published/ampere/ga102/dev_ram.h` contains no `NV_RAMUSERD` at all. Reading the
 *      chip's own header and stopping there is precisely the
 *      `a_table_does_not_decide_behaviour` mistake; letting the dispatch table choose is
 *      the fix.
 *
 * Everything else is one stub (`KernelFifo` is passed as NULL and never dereferenced by
 * the sliced function) and `printf`.
 *
 * ⊘ **This harness never computes an expectation.** It reports what NVIDIA's macros
 * produced; the Rust side compares. An expectation here would be the transcription this
 * oracle exists to remove, moved one file to the left.
 *
 * ============================================================================
 * LICENSING — read before copying anything into this repository
 * ============================================================================
 *
 * Those sources are NVIDIA's, dual-licensed MIT / GPL-2.0. Compiling a slice of them for
 * testing is within the MIT grant. Deliberately, NOTHING from those trees is vendored
 * here: `tests/build.rs` hands the compiler their ABSOLUTE PATHS out of a checkout that
 * already exists beside this repository, and refuses loudly rather than substituting a
 * copy when it is absent. Same arrangement as `vbios_oracle.c`, `gmmu_fmt_oracle.c` and
 * `worksubmit_token_oracle.c`.
 */

#define NVOC_KERNEL_FIFO_H_PRIVATE_ACCESS_ALLOWED

#include <stdio.h>

/* The vendored tree's own headers. The include path is -I'd by tests/build.rs. */
#include "nvmisc.h"
#include "class/clc56f.h"
#include "class/clc7b5.h"
#include "kernel/gpu/fifo/kernel_fifo.h"

/*
 * The `SF_*` accessors, sliced out of the driver's own generated header. They are what
 * turn a `(hi*32+31):(lo*32+0)` field extent into a byte offset.
 */
#include OGKM_SF_ACCESSOR_SLICE

/*
 * `kfifoGetUserdSizeAlign_<HAL>` plus the impl file's own `published/…` includes, sliced
 * by tests/build.rs. `OGKM_USERD_SIZE_FN` is the symbol the driver's dispatch table binds
 * for the chip; `OGKM_USERD_SIZE_FN_NAME` is its spelling, reported below so the artifact
 * says which implementation answered.
 */
#include OGKM_USERD_SIZE_SLICE

/* --------------------------------------------------------------------------- */
/* Encoders — every one of them NVIDIA's macros over NVIDIA's fields            */
/* --------------------------------------------------------------------------- */

/* An INCREMENTING method header — the form `NV_PUSH_nU` and `rm::ce_pushbuffer` write. */
static NvU32 hdr_inc(NvU32 sub, NvU32 method_bytes, NvU32 count)
{
    return DRF_NUM(C56F, _DMA, _INCR_ADDRESS, method_bytes >> 2)
         | DRF_NUM(C56F, _DMA, _INCR_SUBCHANNEL, sub)
         | DRF_NUM(C56F, _DMA, _INCR_COUNT, count)
         | DRF_DEF(C56F, _DMA, _INCR_OPCODE, _VALUE);
}

/* A NON-incrementing header — `NV_PUSH_nN`'s form. */
static NvU32 hdr_nonincr(NvU32 sub, NvU32 method_bytes, NvU32 count)
{
    return DRF_NUM(C56F, _DMA, _NONINCR_ADDRESS, method_bytes >> 2)
         | DRF_NUM(C56F, _DMA, _NONINCR_SUBCHANNEL, sub)
         | DRF_NUM(C56F, _DMA, _NONINCR_COUNT, count)
         | DRF_DEF(C56F, _DMA, _NONINCR_OPCODE, _VALUE);
}

/* An increment-once header. */
static NvU32 hdr_oneincr(NvU32 sub, NvU32 method_bytes, NvU32 count)
{
    return DRF_NUM(C56F, _DMA, _ONEINCR_ADDRESS, method_bytes >> 2)
         | DRF_NUM(C56F, _DMA, _ONEINCR_SUBCHANNEL, sub)
         | DRF_NUM(C56F, _DMA, _ONEINCR_COUNT, count)
         | DRF_DEF(C56F, _DMA, _ONEINCR_OPCODE, _VALUE);
}

/* An immediate-data header: the datum is IN the header and no words follow. */
static NvU32 hdr_immd(NvU32 sub, NvU32 method_bytes, NvU32 data)
{
    return DRF_NUM(C56F, _DMA, _IMMD_ADDRESS, method_bytes >> 2)
         | DRF_NUM(C56F, _DMA, _IMMD_SUBCHANNEL, sub)
         | DRF_NUM(C56F, _DMA, _IMMD_DATA, data)
         | DRF_DEF(C56F, _DMA, _IMMD_OPCODE, _VALUE);
}

/*
 * One GPFIFO entry. `level`/`sync`/`fetch` are passed as the header's own constants.
 *
 * ★★ `fetch` is a parameter and not a constant, and that is a MEASURED correction rather
 * than thoroughness. `GP_ENTRY0_GET` is `31:2`, so bits `1:0` are not the address —
 * `FETCH` is bit 0. Every case in the original sweep used `_UNCONDITIONAL` (bit 0 clear),
 * which made `entry0 & 0xFFFF_FFFC` and a bare `entry0` agree **everywhere**: bite 2 of
 * `scripts/bite_pushbuffer_codec.py` — read the FETCH bit as address bit 0 — was
 * `MISSED BY EVERYTHING`. The `_CONDITIONAL` cases below are what make that mask
 * load-bearing.
 */
static void gp_entry(NvU64 va, NvU32 len_bytes, NvU32 level, NvU32 sync, NvU32 fetch,
                     NvU32 *e0, NvU32 *e1)
{
    *e0 = DRF_NUM(C56F, _GP_ENTRY0, _GET, (NvU32) (va >> 2))
        | DRF_NUM(C56F, _GP_ENTRY0, _FETCH, fetch);
    *e1 = DRF_NUM(C56F, _GP_ENTRY1, _GET_HI, (NvU32) (va >> 32))
        | DRF_NUM(C56F, _GP_ENTRY1, _LENGTH, len_bytes / 4)
        | DRF_NUM(C56F, _GP_ENTRY1, _LEVEL, level)
        | DRF_NUM(C56F, _GP_ENTRY1, _SYNC, sync);
}

/* --------------------------------------------------------------------------- */
/* Emitters                                                                     */
/* --------------------------------------------------------------------------- */

/*
 * ★★★ Both directions are NVIDIA's. `gp_entry` above packs with `DRF_NUM`/`DRF_DEF`; the
 * `dec_*` fields below unpack with `DRF_VAL` — the driver's OWN extractor over the driver's
 * OWN field extents. The Rust side compares its decode against `dec_*` and NOT against the
 * `va`/`len` this harness was called with, which is what makes the sweep past each field's
 * end meaningful: an address of 2^41 cannot survive a 40-bit entry, NVIDIA's extractor says
 * what does survive, and a decoder that reports anything else has invented a field.
 */
static void emit_entry_fetch(const char *name, NvU64 va, NvU32 len, NvU32 level, NvU32 sync,
                             NvU32 fetch, NvU32 raw_or)
{
    NvU32 e0, e1;
    NvU64 dec_va;
    gp_entry(va, len, level, sync, fetch, &e0, &e1);
    /*
     * ⊘ `raw_or` is the ONE place this harness sets a bit NVIDIA has no field for: bit 1
     * of entry0, which `GP_ENTRY0_GET` (31:2) excludes and no `_FETCH` constant covers.
     * It is set so that a decoder reading bits 1:0 as address is visible; `DRF_VAL` below
     * still reports the address without it, which is the assertion.
     */
    e0 |= raw_or;
    dec_va = ((NvU64) DRF_VAL(C56F, _GP_ENTRY0, _GET, e0) << 2)
           | ((NvU64) DRF_VAL(C56F, _GP_ENTRY1, _GET_HI, e1) << 32);
    printf("gpentry %s va=0x%llx len=0x%x entry=0x%08x%08x dec_va=0x%llx dec_len=0x%x "
           "dec_level=%u dec_sync=%u\n",
           name, (unsigned long long) va, (unsigned) len, (unsigned) e1, (unsigned) e0,
           (unsigned long long) dec_va,
           (unsigned) (DRF_VAL(C56F, _GP_ENTRY1, _LENGTH, e1) * 4),
           (unsigned) DRF_VAL(C56F, _GP_ENTRY1, _LEVEL, e1),
           (unsigned) DRF_VAL(C56F, _GP_ENTRY1, _SYNC, e1));
}

static void emit_entry(const char *name, NvU64 va, NvU32 len, NvU32 level, NvU32 sync)
{
    emit_entry_fetch(name, va, len, level, sync, NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL, 0);
}

/*
 * A raw control entry: LENGTH is left zero and entry1's low byte carries an OPCODE.
 * ★ There is no address in it — `GP_ENTRY1_OPCODE` and `GP_ENTRY1_GET_HI` are the SAME
 * eight bits — which is the fact the Rust side's refusal turns on.
 */
static void emit_control_entry(const char *name, NvU32 opcode, NvU32 e0_bits)
{
    NvU32 e1 = DRF_NUM(C56F, _GP_ENTRY1, _OPCODE, opcode);
    printf("gpcontrol %s opcode=%u entry=0x%08x%08x\n", name, (unsigned) opcode,
           (unsigned) e1, (unsigned) e0_bits);
}

/*
 * As with `emit_entry`, the `dec_*` fields are `DRF_VAL` over the driver's own extents.
 * `dec_count` is the raw `METHOD_COUNT` field and is NOT an argument count: what the
 * argument count IS for each `SEC_OP` is exactly the question the Rust side answers, and
 * printing an answer here would be the transcription this oracle removes.
 */
static void emit_header(const char *name, const char *form, NvU32 header, NvU32 sub,
                        NvU32 method_bytes, NvU32 count)
{
    printf("header %s form=%s sub=%u method=0x%x count=%u header=0x%08x dec_secop=%u "
           "dec_tertop=%u dec_addr=0x%x dec_sub=%u dec_count=%u dec_immd=0x%x "
           "dec_addr_old=0x%x dec_count_old=%u\n",
           name, form, (unsigned) sub, (unsigned) method_bytes, (unsigned) count,
           (unsigned) header,
           (unsigned) DRF_VAL(C56F, _DMA, _SEC_OP, header),
           (unsigned) DRF_VAL(C56F, _DMA, _TERT_OP, header),
           (unsigned) (DRF_VAL(C56F, _DMA, _METHOD_ADDRESS, header) * 4),
           (unsigned) DRF_VAL(C56F, _DMA, _METHOD_SUBCHANNEL, header),
           (unsigned) DRF_VAL(C56F, _DMA, _METHOD_COUNT, header),
           (unsigned) DRF_VAL(C56F, _DMA, _IMMD_DATA, header),
           (unsigned) (DRF_VAL(C56F, _DMA, _METHOD_ADDRESS_OLD, header) * 4),
           (unsigned) DRF_VAL(C56F, _DMA, _METHOD_COUNT_OLD, header));
}

/*
 * The host-FIFO semaphore run, decoded by the driver's own extractor — so the Rust side has
 * an independent answer to compare against rather than its own arithmetic.
 */
static void emit_sem_decode(const char *name, const NvU32 *w)
{
    NvU64 addr = ((NvU64) DRF_VAL(C56F, _SEM_ADDR_LO, _OFFSET, w[1]) << 2)
               | ((NvU64) DRF_VAL(C56F, _SEM_ADDR_HI, _OFFSET, w[2]) << 32);
    NvU32 op = DRF_VAL(C56F, _SEM_EXECUTE, _OPERATION, w[5]);
    NvU32 size64 = DRF_VAL(C56F, _SEM_EXECUTE, _PAYLOAD_SIZE, w[5]);
    NvU64 payload = size64 == NVC56F_SEM_EXECUTE_PAYLOAD_SIZE_64BIT
                        ? ((NvU64) DRF_VAL(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, w[3])
                           | ((NvU64) DRF_VAL(C56F, _SEM_PAYLOAD_HI, _PAYLOAD, w[4]) << 32))
                        : (NvU64) DRF_VAL(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, w[3]);
    printf("semdec %s addr=0x%llx payload=0x%llx op=%u release_op=%u payload64=%u\n", name,
           (unsigned long long) addr, (unsigned long long) payload, (unsigned) op,
           (unsigned) NVC56F_SEM_EXECUTE_OPERATION_RELEASE, (unsigned) size64);
}

/* The MEM_OP run, likewise. */
static void emit_memop_decode(const char *name, const NvU32 *w)
{
    NvU64 pdb = ((NvU64) DRF_VAL(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB_ADDR_LO, w[3]) << 12)
              | ((NvU64) DRF_VAL(C56F, _MEM_OP_D, _TLB_INVALIDATE_PDB_ADDR_HI, w[4]) << 32);
    printf("memopdec %s pdb=0x%llx pdb_all=%u sysmembar=%u op=%u inval_op=%u "
           "inval_targeted_op=%u\n",
           name, (unsigned long long) pdb,
           (unsigned) DRF_VAL(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB, w[3]),
           (unsigned) DRF_VAL(C56F, _MEM_OP_A, _TLB_INVALIDATE_SYSMEMBAR, w[1]),
           (unsigned) DRF_VAL(C56F, _MEM_OP_D, _OPERATION, w[4]),
           (unsigned) NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE,
           (unsigned) NVC56F_MEM_OP_D_OPERATION_MMU_TLB_INVALIDATE_TARGETED);
}

/* A blob of pushbuffer words, printed as `blob <name> <n> <w0> <w1> …`. */
static void emit_blob(const char *name, const NvU32 *w, unsigned n)
{
    unsigned i;
    printf("blob %s %u", name, n);
    for (i = 0; i < n; i++)
        printf(" 0x%08x", (unsigned) w[i]);
    printf("\n");
}


/*
 * ★★★ E5 — ONE WHOLE COPY-ENGINE RUN, emitted as the words a guest writes AND as
 * NVIDIA's own reading of them.
 *
 * `sub` is the subchannel, `flags` the `LAUNCH_DMA` word the caller composed out of
 * `DRF_DEF`s, and the operands are the caller's own numbers. The `dec_*` fields are
 * `DRF_VAL` over the driver's extents — so the Rust assertion compares our accumulator's
 * answer against *the driver's* extraction of the same words, never against the number
 * the harness was called with. That is what makes an operand swept past a field's end
 * meaningful: `OFFSET_IN_UPPER_UPPER` is `16:0`, so an address of 2^49 cannot survive it,
 * and NVIDIA's extractor says exactly what does.
 *
 * ⊘ `bind` selects whether the run opens with `SET_OBJECT`. A run without it is the
 * unbound-subchannel refusal, and it has to be built by the same emitter as the bound one
 * or the two are not comparable.
 */
#define CE_PART_BIND    1u
#define CE_PART_OFFSETS 2u
#define CE_PART_LINE    4u
#define CE_PART_LAUNCH  8u
#define CE_PART_ALL     (CE_PART_BIND | CE_PART_OFFSETS | CE_PART_LINE | CE_PART_LAUNCH)

static void emit_ce_run_parts(const char *name, NvU32 sub, unsigned parts, NvU32 obj_class,
                              NvU64 src, NvU64 dst, NvU32 line_len, NvU32 line_count,
                              NvU32 flags, NvU32 dirty_upper)
{
    NvU32 w[32];
    unsigned n = 0, i;
    NvU32 in_up, in_lo, out_up, out_lo;

    if (parts & CE_PART_BIND) {
        w[n++] = hdr_inc(sub, NVC56F_SET_OBJECT, 1);
        w[n++] = DRF_NUM(C56F, _SET_OBJECT, _NVCLASS, obj_class);
    }
    /*
     * ★★★ `dirty_upper` is OR-ed in AFTER `DRF_NUM`, and it is a MEASURED correction.
     *
     * `DRF_NUM` masks its argument to the field, so every operand below bit 49 produced an
     * `_UPPER` word with nothing above bit 16 set — which made `word & 0x1FFFF` and a bare
     * `word` agree EVERYWHERE. The bite "read `OFFSET_IN_UPPER` as a full 32 bits" was
     * MISSED BY EVERYTHING. Hardware does not zero the rest of that register for you.
     * This is the same shape as the `FETCH`-bit gap `emit_entry_fetch` records, one class
     * along, and it is why `dec_src`/`dec_dst` are `DRF_VAL` of the word that was actually
     * written rather than of the number this function was called with.
     */
    in_up  = DRF_NUM(C7B5, _OFFSET_IN_UPPER,  _UPPER, (NvU32) (src >> 32)) | dirty_upper;
    in_lo  = (NvU32) src;
    out_up = DRF_NUM(C7B5, _OFFSET_OUT_UPPER, _UPPER, (NvU32) (dst >> 32)) | dirty_upper;
    out_lo = (NvU32) dst;
    if (parts & CE_PART_OFFSETS) {
        w[n++] = hdr_inc(sub, NVC7B5_OFFSET_IN_UPPER, 4);
        w[n++] = in_up;
        w[n++] = in_lo;
        w[n++] = out_up;
        w[n++] = out_lo;
    }
    if (parts & CE_PART_LINE) {
        w[n++] = hdr_inc(sub, NVC7B5_LINE_LENGTH_IN, 2);
        w[n++] = DRF_NUM(C7B5, _LINE_LENGTH_IN, _VALUE, line_len);
        w[n++] = DRF_NUM(C7B5, _LINE_COUNT, _VALUE, line_count);
    }
    if (parts & CE_PART_LAUNCH) {
        w[n++] = hdr_inc(sub, NVC7B5_LAUNCH_DMA, 1);
        w[n++] = flags;
    }

    printf("cerun %s %u", name, n);
    for (i = 0; i < n; i++)
        printf(" 0x%08x", (unsigned) w[i]);
    printf("\n");
    printf("cedec %s parts=%u class=0x%x src=0x%llx dst=0x%llx len=0x%x count=0x%x "
           "transfer=%u multiline=%u remap=%u srcphys=%u dstphys=%u\n",
           name, parts,
           (unsigned) DRF_VAL(C56F, _SET_OBJECT, _NVCLASS, obj_class),
           (unsigned long long) (((NvU64) DRF_VAL(C7B5, _OFFSET_IN_UPPER, _UPPER, in_up) << 32)
                                 | (NvU64) DRF_VAL(C7B5, _OFFSET_IN_LOWER, _VALUE, in_lo)),
           (unsigned long long) (((NvU64) DRF_VAL(C7B5, _OFFSET_OUT_UPPER, _UPPER, out_up) << 32)
                                 | (NvU64) DRF_VAL(C7B5, _OFFSET_OUT_LOWER, _VALUE, out_lo)),
           (unsigned) DRF_VAL(C7B5, _LINE_LENGTH_IN, _VALUE,
                              DRF_NUM(C7B5, _LINE_LENGTH_IN, _VALUE, line_len)),
           (unsigned) DRF_VAL(C7B5, _LINE_COUNT, _VALUE,
                              DRF_NUM(C7B5, _LINE_COUNT, _VALUE, line_count)),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _REMAP_ENABLE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _SRC_TYPE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _DST_TYPE, flags));
}

/* The whole five-run shape, which is what every accepted case is. */
static void emit_ce_run(const char *name, NvU32 sub, int bind, NvU32 obj_class,
                        NvU64 src, NvU64 dst, NvU32 line_len, NvU32 line_count,
                        NvU32 flags)
{
    emit_ce_run_parts(name, sub, bind ? CE_PART_ALL : (CE_PART_ALL & ~CE_PART_BIND),
                      obj_class, src, dst, line_len, line_count, flags, 0);
}

/*
 * ★★★ ONE WHOLE CE **MEMSET** RUN — the shape RM and UVM actually emit for a constant
 * fill, which is NOT the copy shape with one flag flipped.
 *
 * The differences are structural and each one is a decoder trap:
 *
 * 1. **`SET_REMAP_CONST_A/_B` + `SET_REMAP_COMPONENTS` come first** (`ogkm-580:
 *    src/nvidia/src/kernel/gpu/mem_mgr/channel_utils.c:1029-1033` for RM's scrub map;
 *    `kernel-open/nvidia-uvm/uvm_maxwell_ce.c:379-419` for UVM's three).
 * 2. **`OFFSET_IN_UPPER/_LOWER` are NEVER pushed.** `channelPushMemoryProperties` writes
 *    the source pair only on its `bCeMemcopy` arm (`channel_utils.c:1036-1067`), so a
 *    fill's source registers are simply never latched. A decoder that requires them
 *    refuses every memset the driver sends.
 * 3. **`LINE_LENGTH_IN` counts ELEMENTS, not bytes.** `uvm_hal_maxwell_ce_memset_4` does
 *    `size /= 4` before pushing it, and `memset_common` advances the destination by
 *    `memset_this_time * memset_element_size` (`uvm_maxwell_ce.c:355-372, :391-403`).
 *
 * The `dec_*` line carries the component map read back through NVIDIA's OWN `DRF_VAL`, so
 * the Rust side models the engine from the driver's extraction rather than from ours.
 */
static void emit_ce_fill_run(const char *name, NvU32 sub, NvU64 dst, NvU32 line_len,
                             NvU32 const_a, NvU32 const_b, NvU32 components, NvU32 flags,
                             int push_map, int push_consts)
{
    NvU32 w[32];
    unsigned n = 0, i;
    NvU32 out_up, out_lo;

    w[n++] = hdr_inc(sub, NVC56F_SET_OBJECT, 1);
    w[n++] = DRF_NUM(C56F, _SET_OBJECT, _NVCLASS, AMPERE_DMA_COPY_B);

    /* ⊘ The two constants and the map are pushed independently, because "the map arrived
     * and the constant it selects did not" is a distinct refusal from "no map at all" and
     * a corpus that could not express it would not reach the second one. */
    if (push_consts) {
        w[n++] = hdr_inc(sub, NVC7B5_SET_REMAP_CONST_A, 2);
        w[n++] = DRF_NUM(C7B5, _SET_REMAP_CONST_A, _V, const_a);
        w[n++] = DRF_NUM(C7B5, _SET_REMAP_CONST_B, _V, const_b);
    }
    if (push_map) {
        w[n++] = hdr_inc(sub, NVC7B5_SET_REMAP_COMPONENTS, 1);
        w[n++] = components;
    }

    /* Destination only — two words at 0x408, exactly `NV_PUSH_INC_2U(OFFSET_OUT_UPPER,
     * …, OFFSET_OUT_LOWER, …)`. */
    out_up = DRF_NUM(C7B5, _OFFSET_OUT_UPPER, _UPPER, (NvU32) (dst >> 32));
    out_lo = (NvU32) dst;
    w[n++] = hdr_inc(sub, NVC7B5_OFFSET_OUT_UPPER, 2);
    w[n++] = out_up;
    w[n++] = out_lo;

    /* ⊘ ONE word, not two: the memset path pushes `LINE_LENGTH_IN` alone
     * (`channel_utils.c:828`), so `LINE_COUNT` is never written either. */
    w[n++] = hdr_inc(sub, NVC7B5_LINE_LENGTH_IN, 1);
    w[n++] = DRF_NUM(C7B5, _LINE_LENGTH_IN, _VALUE, line_len);

    w[n++] = hdr_inc(sub, NVC7B5_LAUNCH_DMA, 1);
    w[n++] = flags;

    printf("cerun %s %u", name, n);
    for (i = 0; i < n; i++)
        printf(" 0x%08x", (unsigned) w[i]);
    printf("\n");
    printf("cedec %s parts=%u class=0x%x src=0x0 dst=0x%llx len=0x%x count=0x0 "
           "transfer=%u multiline=%u remap=%u srcphys=%u dstphys=%u "
           "map=%d consts=%d consta=0x%x constb=0x%x compsize=%u numdst=%u "
           "dstx=%u dsty=%u dstz=%u dstw=%u\n",
           name, CE_PART_ALL,
           (unsigned) DRF_VAL(C56F, _SET_OBJECT, _NVCLASS, AMPERE_DMA_COPY_B),
           (unsigned long long) (((NvU64) DRF_VAL(C7B5, _OFFSET_OUT_UPPER, _UPPER, out_up) << 32)
                                 | (NvU64) DRF_VAL(C7B5, _OFFSET_OUT_LOWER, _VALUE, out_lo)),
           (unsigned) DRF_VAL(C7B5, _LINE_LENGTH_IN, _VALUE,
                              DRF_NUM(C7B5, _LINE_LENGTH_IN, _VALUE, line_len)),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _REMAP_ENABLE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _SRC_TYPE, flags),
           (unsigned) DRF_VAL(C7B5, _LAUNCH_DMA, _DST_TYPE, flags),
           push_map, push_consts,
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_CONST_A, _V,
                              DRF_NUM(C7B5, _SET_REMAP_CONST_A, _V, const_a)),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_CONST_B, _V,
                              DRF_NUM(C7B5, _SET_REMAP_CONST_B, _V, const_b)),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, components),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, components),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _DST_X, components),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _DST_Y, components),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _DST_Z, components),
           (unsigned) DRF_VAL(C7B5, _SET_REMAP_COMPONENTS, _DST_W, components));
}

int main(void)
{
    NvU32 userd_size = 0xDEADBEEFu, userd_shift = 0xDEADBEEFu;
    NvU32 w[16];
    unsigned n;

    printf("oracle pushbuffer_abi\n");
    printf("chip %s\n", OGKM_PUSHBUFFER_CHIP);
    printf("userd_hal %s\n", OGKM_USERD_SIZE_FN_NAME);

    /* ---------------------------------------------------------------- USERD */
    /*
     * The driver's own HAL, for the chip the driver's own dispatch table picked. It never
     * dereferences its `KernelFifo *`, so NULL is not a stub standing in for arithmetic.
     */
    OGKM_USERD_SIZE_FN(NULL, &userd_size, &userd_shift);
    printf("userd_size %u\n", (unsigned) userd_size);
    printf("userd_shift %u\n", (unsigned) userd_shift);
    printf("userd_gp_get %u\n", (unsigned) SF_OFFSET(NV_RAMUSERD_GP_GET));
    printf("userd_gp_put %u\n", (unsigned) SF_OFFSET(NV_RAMUSERD_GP_PUT));
    printf("gp_entry_size %u\n", (unsigned) NVC56F_GP_ENTRY__SIZE);
    printf("subchannels %u\n", (unsigned) NVC56F_NUMBER_OF_SUBCHANNELS);

    /* ------------------------------------------------------- the SEC_OP universe */
    /*
     * ★★ Every value the class header enumerates, by name, so the Rust side can quantify
     * its coverage over the DRIVER's list instead of over one an author remembered
     * (`gates_quantified_over_a_list`). A ninth opcode in a later release appears here and
     * turns the comparison red.
     */
    printf("sec_op GRP0_USE_TERT %u\n", (unsigned) NVC56F_DMA_SEC_OP_GRP0_USE_TERT);
    printf("sec_op INC_METHOD %u\n", (unsigned) NVC56F_DMA_SEC_OP_INC_METHOD);
    printf("sec_op GRP2_USE_TERT %u\n", (unsigned) NVC56F_DMA_SEC_OP_GRP2_USE_TERT);
    printf("sec_op NON_INC_METHOD %u\n", (unsigned) NVC56F_DMA_SEC_OP_NON_INC_METHOD);
    printf("sec_op IMMD_DATA_METHOD %u\n", (unsigned) NVC56F_DMA_SEC_OP_IMMD_DATA_METHOD);
    printf("sec_op ONE_INC %u\n", (unsigned) NVC56F_DMA_SEC_OP_ONE_INC);
    printf("sec_op RESERVED6 %u\n", (unsigned) NVC56F_DMA_SEC_OP_RESERVED6);
    printf("sec_op END_PB_SEGMENT %u\n", (unsigned) NVC56F_DMA_SEC_OP_END_PB_SEGMENT);
    printf("tert_op GRP0_INC_METHOD %u\n", (unsigned) NVC56F_DMA_TERT_OP_GRP0_INC_METHOD);
    printf("tert_op GRP2_NON_INC_METHOD %u\n",
           (unsigned) NVC56F_DMA_TERT_OP_GRP2_NON_INC_METHOD);

    /* --------------------------------------------------- the method addresses */
    /*
     * The class headers' own method offsets. The Rust side pins `kayfabe_abi::submit`'s
     * constants against these, so a transcription typo in that module is a red test rather
     * than a method four registers along that does not fault.
     */
    printf("method SET_OBJECT 0x%x\n", (unsigned) NVC56F_SET_OBJECT);
    printf("method SEM_ADDR_LO 0x%x\n", (unsigned) NVC56F_SEM_ADDR_LO);
    printf("method SEM_ADDR_HI 0x%x\n", (unsigned) NVC56F_SEM_ADDR_HI);
    printf("method SEM_PAYLOAD_LO 0x%x\n", (unsigned) NVC56F_SEM_PAYLOAD_LO);
    printf("method SEM_PAYLOAD_HI 0x%x\n", (unsigned) NVC56F_SEM_PAYLOAD_HI);
    printf("method SEM_EXECUTE 0x%x\n", (unsigned) NVC56F_SEM_EXECUTE);
    printf("method MEM_OP_A 0x%x\n", (unsigned) NVC56F_MEM_OP_A);
    printf("method MEM_OP_B 0x%x\n", (unsigned) NVC56F_MEM_OP_B);
    printf("method MEM_OP_C 0x%x\n", (unsigned) NVC56F_MEM_OP_C);
    printf("method MEM_OP_D 0x%x\n", (unsigned) NVC56F_MEM_OP_D);
    printf("method CE_LAUNCH_DMA 0x%x\n", (unsigned) NVC7B5_LAUNCH_DMA);
    printf("method CE_OFFSET_IN_UPPER 0x%x\n", (unsigned) NVC7B5_OFFSET_IN_UPPER);
    printf("method CE_OFFSET_IN_LOWER 0x%x\n", (unsigned) NVC7B5_OFFSET_IN_LOWER);
    printf("method CE_OFFSET_OUT_UPPER 0x%x\n", (unsigned) NVC7B5_OFFSET_OUT_UPPER);
    printf("method CE_OFFSET_OUT_LOWER 0x%x\n", (unsigned) NVC7B5_OFFSET_OUT_LOWER);
    printf("method CE_LINE_LENGTH_IN 0x%x\n", (unsigned) NVC7B5_LINE_LENGTH_IN);
    printf("method CE_LINE_COUNT 0x%x\n", (unsigned) NVC7B5_LINE_COUNT);
    printf("method CE_SET_SEMAPHORE_A 0x%x\n", (unsigned) NVC7B5_SET_SEMAPHORE_A);
    printf("method CE_SET_SEMAPHORE_B 0x%x\n", (unsigned) NVC7B5_SET_SEMAPHORE_B);
    printf("method CE_SET_SEMAPHORE_PAYLOAD 0x%x\n",
           (unsigned) NVC7B5_SET_SEMAPHORE_PAYLOAD);

    /* ------------------------------------------------------------ GPFIFO entries */
    /*
     * ★ A sweep of every address bit the entry can hold, on its own, plus the two bits
     * PAST each field's documented end. The encoder drops those; a decoder that reads them
     * back has invented a field. This is the half a captured trace cannot do — real rings
     * carry a handful of similar-looking addresses that a decoder with a mask one bit too
     * wide agrees with everywhere.
     */
    {
        unsigned i;
        char name[64];
        for (i = 2; i < 42; i++) {
            snprintf(name, sizeof(name), "va_bit%u", i);
            emit_entry(name, 1ull << i, 4, NVC56F_GP_ENTRY1_LEVEL_MAIN,
                       NVC56F_GP_ENTRY1_SYNC_PROCEED);
        }
        for (i = 0; i < 23; i++) {
            snprintf(name, sizeof(name), "len_bit%u", i);
            emit_entry(name, 0x1000, 4u << i, NVC56F_GP_ENTRY1_LEVEL_MAIN,
                       NVC56F_GP_ENTRY1_SYNC_PROCEED);
        }
    }
    emit_entry("ordinary", 0x000A500012300ull, 0x40, NVC56F_GP_ENTRY1_LEVEL_MAIN,
               NVC56F_GP_ENTRY1_SYNC_PROCEED);
    emit_entry("subroutine", 0x2000, 0x18, NVC56F_GP_ENTRY1_LEVEL_SUBROUTINE,
               NVC56F_GP_ENTRY1_SYNC_PROCEED);
    emit_entry("syncwait", 0x2000, 0x18, NVC56F_GP_ENTRY1_LEVEL_MAIN,
               NVC56F_GP_ENTRY1_SYNC_WAIT);
    emit_entry("max_len", 0x1000, 4 * ((1u << 21) - 1), NVC56F_GP_ENTRY1_LEVEL_MAIN,
               NVC56F_GP_ENTRY1_SYNC_PROCEED);
    /* ★★ The two cases that make `GP_ENTRY0_GET`'s `31:2` load-bearing — see gp_entry(). */
    emit_entry_fetch("fetch_conditional", 0x000A500012300ull, 0x40,
                     NVC56F_GP_ENTRY1_LEVEL_MAIN, NVC56F_GP_ENTRY1_SYNC_PROCEED,
                     NVC56F_GP_ENTRY0_FETCH_CONDITIONAL, 0);
    emit_entry_fetch("entry0_bit1", 0x000A500012300ull, 0x40, NVC56F_GP_ENTRY1_LEVEL_MAIN,
                     NVC56F_GP_ENTRY1_SYNC_PROCEED,
                     NVC56F_GP_ENTRY0_FETCH_UNCONDITIONAL, 2);
    emit_entry_fetch("fetch_conditional_and_bit1", 0x000A500012300ull, 0x40,
                     NVC56F_GP_ENTRY1_LEVEL_MAIN, NVC56F_GP_ENTRY1_SYNC_PROCEED,
                     NVC56F_GP_ENTRY0_FETCH_CONDITIONAL, 2);

    /* The four control opcodes — LENGTH zero, so entry1's low byte is OPCODE. */
    emit_control_entry("nop", NVC56F_GP_ENTRY1_OPCODE_NOP, 0xFFFFFFFCu);
    emit_control_entry("illegal", NVC56F_GP_ENTRY1_OPCODE_ILLEGAL, 0xFFFFFFFCu);
    emit_control_entry("gp_crc", NVC56F_GP_ENTRY1_OPCODE_GP_CRC, 0xFFFFFFFCu);
    emit_control_entry("pb_crc", NVC56F_GP_ENTRY1_OPCODE_PB_CRC, 0xFFFFFFFCu);

    /* ------------------------------------------------------------- method headers */
    {
        unsigned i;
        char name[64];
        for (i = 0; i < NVC56F_NUMBER_OF_SUBCHANNELS; i++) {
            snprintf(name, sizeof(name), "inc_sub%u", i);
            emit_header(name, "inc", hdr_inc(i, NVC7B5_LAUNCH_DMA, 1), i,
                        NVC7B5_LAUNCH_DMA, 1);
        }
        /* Every count bit on its own — a 13-bit field, swept past its end by the caller. */
        for (i = 0; i < 13; i++) {
            snprintf(name, sizeof(name), "inc_count_bit%u", i);
            emit_header(name, "inc", hdr_inc(0, NVC56F_SEM_ADDR_LO, 1u << i), 0,
                        NVC56F_SEM_ADDR_LO, 1u << i);
        }
        /* Every method-address bit on its own — a 12-bit dword-indexed field. */
        for (i = 0; i < 12; i++) {
            snprintf(name, sizeof(name), "inc_addr_bit%u", i);
            emit_header(name, "inc", hdr_inc(0, 4u << i, 1), 0, 4u << i, 1);
        }
    }
    emit_header("nonincr", "nonincr", hdr_nonincr(4, NVC7B5_LAUNCH_DMA, 3), 4,
                NVC7B5_LAUNCH_DMA, 3);
    emit_header("oneincr", "oneincr", hdr_oneincr(4, NVC7B5_OFFSET_IN_UPPER, 4), 4,
                NVC7B5_OFFSET_IN_UPPER, 4);
    emit_header("immd", "immd", hdr_immd(4, NVC7B5_LINE_COUNT, 0x1234), 4,
                NVC7B5_LINE_COUNT, 0);
    /* NVC56F_DMA_NOP is the all-zero word — the legacy form with a zero count. */
    emit_header("dma_nop", "legacy", NVC56F_DMA_NOP, 0, 0, 0);
    /*
     * ★ The remaining `SEC_OP`s, built with the class header's own `SEC_OP` field so the
     * Rust side's refusals are judged against inputs the DRIVER's macros produced. The
     * legacy pair carries a non-zero `COUNT_OLD`; the two undefined encodings carry a body
     * that looks perfectly ordinary, which is the point — nothing about them is visibly
     * wrong and the codec must refuse them anyway.
     */
    emit_header("end_pb_segment", "end",
                DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_END_PB_SEGMENT)
                    | DRF_NUM(C56F, _DMA, _METHOD_COUNT, 5),
                0, 0, 5);
    emit_header("reserved6", "reserved",
                DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_RESERVED6)
                    | DRF_NUM(C56F, _DMA, _METHOD_COUNT, 5)
                    | DRF_NUM(C56F, _DMA, _METHOD_ADDRESS, NVC7B5_LAUNCH_DMA >> 2),
                0, NVC7B5_LAUNCH_DMA, 5);
    emit_header("legacy_grp0_inc", "legacy",
                DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_GRP0_USE_TERT)
                    | DRF_NUM(C56F, _DMA, _TERT_OP, NVC56F_DMA_TERT_OP_GRP0_INC_METHOD)
                    | DRF_NUM(C56F, _DMA, _METHOD_ADDRESS_OLD, NVC7B5_LAUNCH_DMA >> 2)
                    | DRF_NUM(C56F, _DMA, _METHOD_COUNT_OLD, 3),
                0, NVC7B5_LAUNCH_DMA, 3);
    emit_header("legacy_grp2_noninc", "legacy",
                DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_GRP2_USE_TERT)
                    | DRF_NUM(C56F, _DMA, _TERT_OP, NVC56F_DMA_TERT_OP_GRP2_NON_INC_METHOD)
                    | DRF_NUM(C56F, _DMA, _METHOD_ADDRESS_OLD, NVC7B5_LAUNCH_DMA >> 2)
                    | DRF_NUM(C56F, _DMA, _METHOD_COUNT_OLD, 3),
                0, NVC7B5_LAUNCH_DMA, 3);
    emit_header("grp0_set_subdev_mask", "subdev",
                DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_GRP0_USE_TERT)
                    | DRF_NUM(C56F, _DMA, _TERT_OP,
                              NVC56F_DMA_TERT_OP_GRP0_SET_SUB_DEV_MASK)
                    | DRF_NUM(C56F, _DMA, _SET_SUBDEVICE_MASK_VALUE, 0xFFF),
                0, 0, 0);
    {
        unsigned t;
        char name[64];
        for (t = 1; t < 4; t++) {
            snprintf(name, sizeof(name), "grp2_tert%u", t);
            emit_header(name, "undefined",
                        DRF_NUM(C56F, _DMA, _SEC_OP, NVC56F_DMA_SEC_OP_GRP2_USE_TERT)
                            | DRF_NUM(C56F, _DMA, _TERT_OP, t)
                            | DRF_NUM(C56F, _DMA, _METHOD_ADDRESS, NVC7B5_LAUNCH_DMA >> 2),
                        0, NVC7B5_LAUNCH_DMA, 0);
        }
    }

    /* --------------------------------------------------------------- blobs */
    /*
     * ★★★ The three real method runs, word for word, built with the class headers' own
     * constants. These are the bytes an `AMPERE_CHANNEL_GPFIFO_A` guest writes, and the
     * E4 acceptance is that `read_pushbuffer` over them yields the same runs at the same
     * word offsets.
     */

    /* 1. The host-FIFO semaphore release — five words, one incrementing run. This is
     *    `rm::sem_pushbuffer`'s exact shape, the one a real GA106 executed at rung R15. */
    n = 0;
    w[n++] = hdr_inc(0, NVC56F_SEM_ADDR_LO, 5);
    w[n++] = DRF_NUM(C56F, _SEM_ADDR_LO, _OFFSET, (NvU32) (0x000A500012300ull >> 2));
    w[n++] = DRF_NUM(C56F, _SEM_ADDR_HI, _OFFSET, (NvU32) (0x000A500012300ull >> 32));
    w[n++] = DRF_NUM(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, 0xBEEF5EA1u);
    w[n++] = DRF_NUM(C56F, _SEM_PAYLOAD_HI, _PAYLOAD, 0u);
    w[n++] = DRF_DEF(C56F, _SEM_EXECUTE, _OPERATION, _RELEASE)
           | DRF_DEF(C56F, _SEM_EXECUTE, _PAYLOAD_SIZE, _32BIT)
           | DRF_DEF(C56F, _SEM_EXECUTE, _RELEASE_WFI, _DIS)
           | DRF_DEF(C56F, _SEM_EXECUTE, _RELEASE_TIMESTAMP, _DIS);
    emit_blob("sem_release_32", w, n);
    emit_sem_decode("sem_release_32", w);

    /* The same run with a 64-bit payload, so the Rust side's payload-size branch has a
     * case on each side rather than one and an assumption. */
    w[3] = DRF_NUM(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, 0x01234567u);
    w[4] = DRF_NUM(C56F, _SEM_PAYLOAD_HI, _PAYLOAD, 0x89ABCDEFu);
    w[5] = DRF_DEF(C56F, _SEM_EXECUTE, _OPERATION, _RELEASE)
         | DRF_DEF(C56F, _SEM_EXECUTE, _PAYLOAD_SIZE, _64BIT);
    emit_blob("sem_release_64", w, n);
    emit_sem_decode("sem_release_64", w);

    /*
     * ★★★ A **32-bit** release with a DIRTY `SEM_PAYLOAD_HI`.
     *
     * ⊘ MEASURED GAP, 2026-08-02. Every release above left `PAYLOAD_HI` at zero, so
     * `payload_lo` and `payload_lo | payload_hi << 32` were the same number and bite 20 of
     * `scripts/bite_pushbuffer_codec.py` — drop the payload-size branch entirely — was
     * `MISSED BY EVERYTHING`. Hardware does not zero that word for you; whatever the guest
     * last wrote is in it. This case is what makes the branch load-bearing.
     */
    w[3] = DRF_NUM(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, 0xBEEF5EA1u);
    w[4] = DRF_NUM(C56F, _SEM_PAYLOAD_HI, _PAYLOAD, 0xDEADBEEFu);
    w[5] = DRF_DEF(C56F, _SEM_EXECUTE, _OPERATION, _RELEASE)
         | DRF_DEF(C56F, _SEM_EXECUTE, _PAYLOAD_SIZE, _32BIT);
    emit_blob("sem_release_32_dirty_hi", w, n);
    emit_sem_decode("sem_release_32_dirty_hi", w);

    /*
     * ⊘ …and **every** value of `SEM_EXECUTE_OPERATION`, not a sample.
     *
     * MEASURED GAP, 2026-08-02: the first version emitted operations 0, 1, 2 and 6, and
     * bite 15 — mask `OPERATION` to one bit instead of three — was `MISSED BY EVERYTHING`,
     * because under that mask only operations **3** and **5** change answer and neither was
     * in the sample. `gates_quantified_over_a_list`: the field is `2:0`, so the universe is
     * the eight values of `2:0` and it is swept as such rather than as the six the header
     * happens to name.
     */
    {
        unsigned op;
        char name[64];
        for (op = 0; op < 8; op++) {
            snprintf(name, sizeof(name), "sem_op%u", op);
            w[3] = DRF_NUM(C56F, _SEM_PAYLOAD_LO, _PAYLOAD, 0x01234567u);
            w[4] = DRF_NUM(C56F, _SEM_PAYLOAD_HI, _PAYLOAD, 0x89ABCDEFu);
            w[5] = DRF_NUM(C56F, _SEM_EXECUTE, _OPERATION, op);
            emit_blob(name, w, n);
            emit_sem_decode(name, w);
        }
    }

    /* 2. The MMU TLB invalidate — four words, one incrementing run, MEM_OP_A..D. */
    n = 0;
    w[n++] = hdr_inc(0, NVC56F_MEM_OP_A, 4);
    w[n++] = DRF_DEF(C56F, _MEM_OP_A, _TLB_INVALIDATE_SYSMEMBAR, _EN);
    w[n++] = 0;
    w[n++] = DRF_DEF(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB, _ONE)
           | DRF_DEF(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB_APERTURE, _VID_MEM)
           | DRF_NUM(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB_ADDR_LO,
                     (NvU32) (0x0000034017FF000ull >> 12));
    w[n++] = DRF_NUM(C56F, _MEM_OP_D, _TLB_INVALIDATE_PDB_ADDR_HI,
                     (NvU32) (0x0000034017FF000ull >> 32))
           | DRF_DEF(C56F, _MEM_OP_D, _OPERATION, _MMU_TLB_INVALIDATE);
    emit_blob("tlb_invalidate_membar", w, n);
    emit_memop_decode("tlb_invalidate_membar", w);

    w[1] = DRF_DEF(C56F, _MEM_OP_A, _TLB_INVALIDATE_SYSMEMBAR, _DIS);
    emit_blob("tlb_invalidate_nomembar", w, n);
    emit_memop_decode("tlb_invalidate_nomembar", w);

    /* ⊘ PDB_ALL carries no address at all. */
    w[3] = DRF_DEF(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB, _ALL);
    emit_blob("tlb_invalidate_pdb_all", w, n);
    emit_memop_decode("tlb_invalidate_pdb_all", w);

    /* ⊘ …and the same run asking for something else entirely. */
    w[3] = DRF_DEF(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB, _ONE)
         | DRF_NUM(C56F, _MEM_OP_C, _TLB_INVALIDATE_PDB_ADDR_LO,
                   (NvU32) (0x0000034017FF000ull >> 12));
    w[4] = DRF_DEF(C56F, _MEM_OP_D, _OPERATION, _L2_FLUSH_DIRTY);
    emit_blob("mem_op_l2_flush", w, n);
    emit_memop_decode("mem_op_l2_flush", w);

    /* 3. The copy-engine pushbuffer, method for method the shape
     *    `kayfabe_isolate_host::rm::ce_pushbuffer` builds — the one a real GA106 executed
     *    at rung R17. ★★★ Note what it is: FIVE separate runs, and `LAUNCH_DMA` is the
     *    last of them carrying nothing but flags. Neither operand, nor the length, is in
     *    it. That is the whole reason `Ga10xPushbuffer` refuses to decode a
     *    `PushMethod::CeLaunchDma` at the per-method seam. */
    n = 0;
    w[n++] = hdr_inc(4, NVC56F_SET_OBJECT, 1);
    w[n++] = AMPERE_DMA_COPY_B;
    emit_blob("ce_set_object", w, n);

    n = 0;
    w[n++] = hdr_inc(4, NVC7B5_OFFSET_IN_UPPER, 4);
    w[n++] = DRF_NUM(C7B5, _OFFSET_IN_UPPER, _UPPER, (NvU32) (0x00A5000011000ull >> 32));
    w[n++] = (NvU32) 0x00A5000011000ull;
    w[n++] = DRF_NUM(C7B5, _OFFSET_OUT_UPPER, _UPPER, (NvU32) (0x00A5000022000ull >> 32));
    w[n++] = (NvU32) 0x00A5000022000ull;
    emit_blob("ce_offsets", w, n);

    n = 0;
    w[n++] = hdr_inc(4, NVC7B5_LINE_LENGTH_IN, 2);
    w[n++] = 0x1000;
    w[n++] = 1;
    emit_blob("ce_line", w, n);

    n = 0;
    w[n++] = hdr_inc(4, NVC7B5_SET_SEMAPHORE_A, 3);
    w[n++] = DRF_NUM(C7B5, _SET_SEMAPHORE_A, _UPPER, (NvU32) (0x00A5000033000ull >> 32));
    w[n++] = (NvU32) 0x00A5000033000ull;
    w[n++] = 0xBEEF5EA1u;
    emit_blob("ce_semaphore", w, n);

    n = 0;
    w[n++] = hdr_inc(4, NVC7B5_LAUNCH_DMA, 1);
    w[n++] = DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _NON_PIPELINED)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _FLUSH_ENABLE, _TRUE)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _SEMAPHORE_TYPE, _RELEASE_ONE_WORD_SEMAPHORE)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_MEMORY_LAYOUT, _PITCH)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_MEMORY_LAYOUT, _PITCH)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _FALSE)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _VIRTUAL)
           | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _VIRTUAL);
    emit_blob("ce_launch_dma", w, n);

    /* The `LAUNCH_DMA` flag values the port names, each on its own, so a constant that
     * moved is a red test rather than a differently-shaped copy. */
    printf("launch NON_PIPELINED 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _NON_PIPELINED));
    printf("launch FLUSH_ENABLE 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _FLUSH_ENABLE, _TRUE));
    printf("launch SEMAPHORE_ONE_WORD 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _SEMAPHORE_TYPE,
                              _RELEASE_ONE_WORD_SEMAPHORE));
    printf("launch SRC_PITCH 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_MEMORY_LAYOUT, _PITCH));
    printf("launch DST_PITCH 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _DST_MEMORY_LAYOUT, _PITCH));
    printf("launch MULTI_LINE_DISABLE 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _FALSE));
    printf("launch SRC_VIRTUAL 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _VIRTUAL));
    printf("launch DST_VIRTUAL 0x%08x\n",
           (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _VIRTUAL));

    /* The SET_OBJECT class mask, read back through the driver's own extractor: a word
     * with EVERY bit set, asked what class it names. */
    printf("set_object_nvclass 0x%08x\n",
           (unsigned) DRF_VAL(C56F, _SET_OBJECT, _NVCLASS, 0xFFFFFFFFu));
    printf("class AMPERE_CHANNEL_GPFIFO_A 0x%x\n", (unsigned) AMPERE_CHANNEL_GPFIFO_A);
    printf("class AMPERE_DMA_COPY_B 0x%x\n", (unsigned) AMPERE_DMA_COPY_B);


    /* ------------------------------------------------- E5: whole copy-engine runs */
    /*
     * ★★★ The accumulator's acceptance. Every case below is built by ONE emitter, so a
     * refusal and an acceptance differ by exactly the field named in the case name.
     */
    {
        NvU32 base = DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _NON_PIPELINED)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _FLUSH_ENABLE, _TRUE)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _SEMAPHORE_TYPE, _RELEASE_ONE_WORD_SEMAPHORE)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_MEMORY_LAYOUT, _PITCH)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_MEMORY_LAYOUT, _PITCH)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _FALSE)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _VIRTUAL)
                   | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _VIRTUAL);
        unsigned i;
        char name[64];

        /* The shape a real GA106 executed at rung R17, on every subchannel the class
         * enumerates — because the accumulator is per-subchannel and a codec that used
         * subchannel 0 for everything would pass a one-subchannel corpus. */
        for (i = 0; i < NVC56F_NUMBER_OF_SUBCHANNELS; i++) {
            snprintf(name, sizeof(name), "copy_sub%u", i);
            emit_ce_run(name, i, 1, AMPERE_DMA_COPY_B,
                        0x00A5000011000ull, 0x00A5000022000ull, 0x1000, 1, base);
        }

        /* ★ Every address bit the 17-bit `_UPPER` field can hold, and the two past its
         * end. A decoder with a 32-bit mask agrees with a real trace everywhere and
         * disagrees here; a decoder with the GPFIFO entry's 8-bit mask — the nearby
         * number a reader is most likely to reuse — disagrees from bit 40. */
        for (i = 0; i < 20; i++) {
            snprintf(name, sizeof(name), "copy_srcbit%u", 32 + i);
            emit_ce_run(name, 3, 1, AMPERE_DMA_COPY_B,
                        1ull << (32 + i), 0x2000, 0x40, 1, base);
            snprintf(name, sizeof(name), "copy_dstbit%u", 32 + i);
            emit_ce_run(name, 3, 1, AMPERE_DMA_COPY_B,
                        0x2000, 1ull << (32 + i), 0x40, 1, base);
        }

        /* The operand-FORM matrix — all four corners, since `src_is_virtual` and
         * `dst_is_virtual` feed two different decisions in `kayfabe-fwd`. */
        emit_ce_run("copy_srcphys", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_SRC_TYPE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _PHYSICAL));
        emit_ce_run("copy_dstphys", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DST_TYPE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _PHYSICAL));
        emit_ce_run("copy_bothphys", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1,
                    (base & ~(DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_SRC_TYPE)
                              | DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DST_TYPE)))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _PHYSICAL)
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _PHYSICAL));
        /* PIPELINED is as much a copy as NON_PIPELINED — the field is a scheduling hint,
         * and a decoder that keyed on the exact value the isolate's encoder writes would
         * refuse every copy a real driver sends. */
        emit_ce_run("copy_pipelined", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DATA_TRANSFER_TYPE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _PIPELINED));

        /* ⊘ The four refusals, each one field away from `copy_sub2`. */
        emit_ce_run("refuse_transfer_none", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000,
                    0x1000, 1,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DATA_TRANSFER_TYPE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _NONE));
        emit_ce_run("refuse_multiline", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000,
                    0x1000, 4,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_MULTI_LINE_ENABLE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _TRUE));
        emit_ce_run("refuse_remap", 2, 1, AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1,
                    (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_REMAP_ENABLE))
                        | DRF_DEF(C7B5, _LAUNCH_DMA, _REMAP_ENABLE, _TRUE));
        /* ⊘ Bound to the CHANNEL class instead: a subchannel's method addresses mean
         * whatever its bound object says they mean, so 0x300 there is not `LAUNCH_DMA`,
         * and a codec that fired anyway would decode another class's method as a copy. */
        emit_ce_run("refuse_wrong_object", 2, 1, AMPERE_CHANNEL_GPFIFO_A, 0x11000, 0x22000,
                    0x1000, 1, base);
        /* ⊘ …and with nothing bound at all. */
        emit_ce_run("refuse_unbound", 2, 0, AMPERE_DMA_COPY_B, 0x11000, 0x22000,
                    0x1000, 1, base);
        /*
         * ⊘⊘ THE TWO CASES A `unwrap_or_default()` SURVIVES, and they are here because it
         * DID: bite "fill the operands with `unwrap_or_default()` instead of refusing" was
         * MISSED BY EVERYTHING in the first sweep. Every prefix of a complete run stops
         * before the launch, so no prefix ever asked a launch to fire without operands.
         * These two do — a launch that is BOUND and has nothing to copy with.
         */
        emit_ce_run_parts("refuse_no_operands", 2, CE_PART_BIND | CE_PART_LAUNCH,
                          AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1, base, 0);
        emit_ce_run_parts("refuse_no_length", 2,
                          CE_PART_BIND | CE_PART_OFFSETS | CE_PART_LAUNCH,
                          AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1, base, 0);
        emit_ce_run_parts("refuse_no_offsets", 2,
                          CE_PART_BIND | CE_PART_LINE | CE_PART_LAUNCH,
                          AMPERE_DMA_COPY_B, 0x11000, 0x22000, 0x1000, 1, base, 0);

        /*
         * ★★★ …and the accepted run whose `_UPPER` words carry EVERY bit above the
         * field's end. `OFFSET_IN_UPPER_UPPER` is `16:0`; a decoder reading the whole word
         * agrees with every case above and disagrees here by 2^32 × 0x7FFF.
         */
        for (i = 17; i < 32; i++) {
            snprintf(name, sizeof(name), "copy_dirtyupper%u", i);
            emit_ce_run_parts(name, 2, CE_PART_ALL, AMPERE_DMA_COPY_B,
                              0x00A5000011000ull, 0x00A5000022000ull, 0x1000, 1, base,
                              1u << i);
        }

        /*
         * ==========================================================================
         * ★★★ THE CONSTANT FILL — the shape `memmgrTestCeUtils`' `memmgrMemSet` sends,
         * and every neighbour of it, built out of the driver's own component maps.
         * ==========================================================================
         */
        {
            NvU32 fill_base = (base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_REMAP_ENABLE))
                            | DRF_DEF(C7B5, _LAUNCH_DMA, _REMAP_ENABLE, _TRUE);
            /* RM's own scrub/memset map, verbatim (`channel_utils.c:1031-1033`). */
            NvU32 map_rm = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_A)
                         | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _ONE)
                         | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* UVM's `memset_1` — same widths, CONST_B (`uvm_maxwell_ce.c:383-385`). */
            NvU32 map_uvm1 = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_B)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _ONE)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* UVM's `memset_4` — a FOUR-byte element (`uvm_maxwell_ce.c:398-400`). */
            NvU32 map_uvm4 = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_B)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _FOUR)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* A TWO-byte element: not emitted by either driver in reach, but the field
             * enumerates it and it is the only period between 1 and 4. */
            NvU32 map_two = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_A)
                          | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _TWO)
                          | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* Two 2-byte components: a 4-byte element assembled from BOTH constants, so
             * a decoder that read only CONST_A gets the top half wrong. */
            NvU32 map_ab = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_A)
                         | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_Y, _CONST_B)
                         | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _TWO)
                         | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _TWO);
            /* UVM's `memset_8` — an EIGHT-byte period (`uvm_maxwell_ce.c:414-417`). */
            NvU32 map_uvm8 = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_A)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_Y, _CONST_B)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _FOUR)
                           | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _TWO);
            /* A THREE-byte element — a period that divides neither 4 nor 8. */
            NvU32 map_three = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _CONST_A)
                            | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _THREE)
                            | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* A selector naming a SOURCE component: a swizzle, not a fill. */
            NvU32 map_src = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _SRC_X)
                          | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _FOUR)
                          | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);
            /* `NO_WRITE` — the engine skips the component and the old bytes survive. */
            NvU32 map_nowrite = DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _DST_X, _NO_WRITE)
                              | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _COMPONENT_SIZE, _FOUR)
                              | DRF_DEF(C7B5, _SET_REMAP_COMPONENTS, _NUM_DST_COMPONENTS, _ONE);

            /*
             * ⊘ Every accepted pattern below is deliberately NON-BYTE-UNIFORM where its
             * element can express one. `memmgrTestCeUtils` itself memsets with **zero**
             * (`ogkm-580: mem_mgr.c:463`), and a corpus that only used its value would be
             * satisfied by a decoder that dropped the pattern entirely.
             */
            emit_ce_fill_run("fill_rm_scrubmap", 2, 0x22000, 0x1000,
                             0x0403UL | 0x02010000UL, 0xBBBBBBBBu, map_rm, fill_base, 1, 1);
            emit_ce_fill_run("fill_uvm1", 2, 0x22000, 0x1000,
                             0xAAAAAAAAu, 0x0403UL | 0x02010000UL, map_uvm1, fill_base, 1, 1);
            emit_ce_fill_run("fill_uvm4", 2, 0x22000, 0x400,
                             0xAAAAAAAAu, 0x0403UL | 0x02010000UL, map_uvm4, fill_base, 1, 1);
            emit_ce_fill_run("fill_two", 2, 0x22000, 0x800,
                             0x0403UL | 0x02010000UL, 0xBBBBBBBBu, map_two, fill_base, 1, 1);
            emit_ce_fill_run("fill_consta_and_constb", 2, 0x22000, 0x400,
                             0x0201u, 0x0403u, map_ab, fill_base, 1, 1);
            /* ★ The actual boot case: RM's map, pattern ZERO, four BYTES — one element. */
            emit_ce_fill_run("fill_memmgrtestceutils", 2, 0x22000, 4,
                             0x0u, 0x0u, map_rm, fill_base, 1, 1);
            /* ★ A PHYSICAL destination — `memmgrTestCeUtils` reaches this arm whenever
             * `bUseVasForCeCopy` is false, and it is a different downstream decision. */
            emit_ce_fill_run("fill_dstphys", 2, 0x22000, 0x1000,
                             0x0403UL | 0x02010000UL, 0u, map_rm,
                             (fill_base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DST_TYPE))
                                 | DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _PHYSICAL), 1, 1);
            /* ★ A 4-byte element on a 4-ALIGNED destination is accepted; the unaligned
             * twin below is not, and the two differ only in `OFFSET_OUT_LOWER`. */
            emit_ce_fill_run("fill_uvm4_aligned", 2, 0x22004, 0x40,
                             0u, 0x0403UL | 0x02010000UL, map_uvm4, fill_base, 1, 1);

            /* ⊘ The refusals, each naming ONE thing the decoder cannot say. */
            emit_ce_fill_run("refusefill_nomap", 2, 0x22000, 0x1000,
                             0x11111111u, 0x22222222u, map_rm, fill_base, 0, 1);
            emit_ce_fill_run("refusefill_noconst", 2, 0x22000, 0x1000,
                             0x11111111u, 0x22222222u, map_rm, fill_base, 1, 0);
            emit_ce_fill_run("refusefill_srccomponent", 2, 0x22000, 0x400,
                             0x11111111u, 0x22222222u, map_src, fill_base, 1, 1);
            emit_ce_fill_run("refusefill_nowrite", 2, 0x22000, 0x400,
                             0x11111111u, 0x22222222u, map_nowrite, fill_base, 1, 1);
            emit_ce_fill_run("refusefill_period8", 2, 0x22000, 0x200,
                             0x11111111u, 0x22222222u, map_uvm8, fill_base, 1, 1);
            emit_ce_fill_run("refusefill_period3", 2, 0x22000, 0x400,
                             0x11111111u, 0x22222222u, map_three, fill_base, 1, 1);
            emit_ce_fill_run("refusefill_unaligned4", 2, 0x22002, 0x40,
                             0u, 0x0403UL | 0x02010000UL, map_uvm4, fill_base, 1, 1);
            emit_ce_fill_run("refusefill_multiline", 2, 0x22000, 0x400,
                             0x11111111u, 0x22222222u, map_rm,
                             (fill_base & ~DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_MULTI_LINE_ENABLE))
                                 | DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _TRUE),
                             1, 1);

            printf("remap COMPONENT_SIZE_ONE %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_COMPONENT_SIZE_ONE);
            printf("remap COMPONENT_SIZE_FOUR %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_COMPONENT_SIZE_FOUR);
            printf("remap NUM_DST_COMPONENTS_ONE %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_NUM_DST_COMPONENTS_ONE);
            printf("remap NUM_DST_COMPONENTS_FOUR %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_NUM_DST_COMPONENTS_FOUR);
            printf("remap DST_X_CONST_A %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_DST_X_CONST_A);
            printf("remap DST_X_CONST_B %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_DST_X_CONST_B);
            printf("remap DST_X_NO_WRITE %u\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS_DST_X_NO_WRITE);
            printf("remap SET_REMAP_CONST_A 0x%x\n", (unsigned) NVC7B5_SET_REMAP_CONST_A);
            printf("remap SET_REMAP_CONST_B 0x%x\n", (unsigned) NVC7B5_SET_REMAP_CONST_B);
            printf("remap SET_REMAP_COMPONENTS 0x%x\n",
                   (unsigned) NVC7B5_SET_REMAP_COMPONENTS);
        }

        printf("launch TRANSFER_MASK 0x%08x\n",
               (unsigned) DRF_SHIFTMASK(NVC7B5_LAUNCH_DMA_DATA_TRANSFER_TYPE));
        printf("launch TRANSFER_NONE 0x%08x\n",
               (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _DATA_TRANSFER_TYPE, _NONE));
        printf("launch MULTI_LINE_ENABLE 0x%08x\n",
               (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _MULTI_LINE_ENABLE, _TRUE));
        printf("launch REMAP_ENABLE 0x%08x\n",
               (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _REMAP_ENABLE, _TRUE));
        printf("launch SRC_PHYSICAL 0x%08x\n",
               (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _SRC_TYPE, _PHYSICAL));
        printf("launch DST_PHYSICAL 0x%08x\n",
               (unsigned) DRF_DEF(C7B5, _LAUNCH_DMA, _DST_TYPE, _PHYSICAL));
        printf("offset_upper_mask 0x%08x\n",
               (unsigned) DRF_VAL(C7B5, _OFFSET_IN_UPPER, _UPPER, 0xFFFFFFFFu));
    }

    printf("end\n");
    return 0;
}
