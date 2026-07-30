# The doorbell mapping, and whether it survives the sandbox

**Status: rung 3 was NOT BUILT and NOTHING here was measured on hardware.** Both bench
boxes went down before a line of it was written. This file exists so the analysis is not
re-derived from scratch, and every claim in it carries a label saying what kind of claim it
is. There are exactly three labels and they are used strictly:

| label | meaning |
|---|---|
| **MEASURED** | observed on an RTX 3060 / 580.159.04 by rung 1 or rung 2, with the output quoted |
| **SOURCE** | read out of `research_clones/ogkm-580.159.04/`, cited to file:line. Not run. |
| **UNKNOWN** | nobody knows, and this file says so rather than guessing |

Nothing below is MEASURED for the doorbell itself. The MEASURED rows are facts from the two
landed rungs that the doorbell path either inherits or is explicitly *not* covered by — and
telling those two apart is most of the value here.

---

## 1. What the doorbell is

**SOURCE.** A doorbell is a 32-bit store of a channel's work-submit token into a mapped
window on the GPU, and there is no ioctl that stands in for it. The window is the CPU
mapping of an `AMPERE_USERMODE_A` object (class `0xc561`,
`ogkm-580: src/common/sdk/nvidia/inc/class/clc561.h:27`), which is allocated under the
subdevice and takes no alloc parameters. Its register layout is the Volta one — `clc561.h`
defines only the class id, and the offsets live at
`ogkm-580: src/common/sdk/nvidia/inc/class/clc361.h:30-33`:

- window size `NVC361_NV_USERMODE__SIZE` = 65536 (`clc361.h:31`)
- **`NVC361_NOTIFY_CHANNEL_PENDING` = `0x90`** (`clc361.h:33`) — the doorbell

Both constants are already in `kayfabe_abi::submit` with these citations, landed at rung 1.

### ★ A trap: there are two doorbells and only one of them is this one

**SOURCE.** `NV_VIRTUAL_FUNCTION_PRIV_DOORBELL` = `0x2200`
(`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_vm.h:126`) is a *different*
register, in the virtual-function BAR0 aperture, and is the SR-IOV/privileged path. The
usermode-class doorbell this port needs is `0x90` **within the usermode window**, not
`0x2200` within BAR0. They are both plausibly "the doorbell" to a reader who greps, and
only one of them is reachable by an unprivileged process (§3).

---

## 2. How the mapping is made — this part is MEASURED, at rung 2

**MEASURED.** Rung 2 landed `RmConnection::map_cpu`, which CPU-maps an RM memory object,
and proved it on hardware for a channel's GPFIFO ring and USERD. The doorbell needs exactly
the same four-step protocol, and all four steps are already measured:

1. the `NV_ESC_RM_MAP_MEMORY` escape goes on the **control** node while the `mmap` goes on
   the **device** node — issuing it on the device node was bitten and returned EINVAL;
2. the descriptor named inside the escape must be of the matching **kind** — naming the
   control node's descriptor was bitten and returned `0x1F`;
3. the mapping context is **one-shot per descriptor**, so every mapping needs a freshly
   opened node — registering twice on one descriptor was bitten and returned
   `0x63 NV_ERR_STATE_IN_USE`;
4. the `mmap` offset must be 0 and the length exact — mapping half the object was bitten
   and returned ENXIO.

So the *mechanism* is not the risk. What is unproven for the doorbell is which **address
range** the mapping covers and who is allowed to cover it, which is §3, and what
**cache attribute** it gets, which is §4.

---

## 3. ★★★ Does it work from inside the sandbox? SOURCE says yes, and says why

This is the question the coordinator flagged as unanswered and expensive to re-reach. It is
now answered **from source**. It is still not measured.

### 3.1 The isolate is not an administrator, so it does not take the fast path

**SOURCE.** Every CPU mapping of a device address goes through `RmValidateMmapRequest`
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2023-2054`), whose **first statement**
is:

```c
if (osIsAdministrator())
{
    *pProtection = NV_PROTECT_READ_WRITE;
    return NV_OK;
}
```

and `osIsAdministrator()` → `os_is_administrator()`
(`ogkm-580: src/nvidia/arch/nvalloc/unix/src/os.c:614-617`) → `NV_IS_SUSER()`
(`ogkm-580: kernel-open/nvidia/os-interface.c:378-381`) → **`capable(CAP_SYS_ADMIN)`**
(`ogkm-580: kernel-open/common/inc/nv-linux.h:537`).

The isolate surrenders every capability, so this is **false** for it, and it takes the
validation path instead. ★ Note `capable()` is a check against the *initial* user
namespace, so entering a user namespace does not restore it either.

### 3.2 ★★ The validation path WHITELISTS the usermode window, read-write

**SOURCE.** `subdeviceCtrlCmdValidateMemMapRequest_IMPL`
(`ogkm-580: src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:2887-2963`)
walks BAR0 range by range for a non-admin caller:

| range | outcome for a non-admin |
|---|---|
| the timer's BAR0 range | `NV_OK`, but downgraded to `NV_PROTECT_READABLE` |
| **`kfifoGetUsermodeMapInfo_HAL`** | **`NV_OK`, protection left `READ_WRITE`** |
| the master-control BAR0 range | `NV_OK`, downgraded to `NV_PROTECT_READABLE` |
| anything else in BAR0 | `NV_ERR_PROTECTION_FAULT` |
| within the framebuffer | `NV_OK`, read-write ("See bug 1784955") |
| anything else | `NV_ERR_PROTECTION_FAULT` |

and `kfifoGetUsermodeMapInfo_GV100`
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/volta/kernel_fifo_gv100.c:155-174`) returns
`NV_REG_BASE_USERMODE` with size `DRF_SIZE(NVC361)` — i.e. exactly the 64 KiB window from
§1.

**So the usermode window is one of exactly two BAR0 ranges an unprivileged process may map
read-write, and it is deliberately so:** that whitelist is what lets an ordinary CUDA
process ring its own doorbell, which is the entire purpose of the Volta+ usermode class.
The prediction is therefore **the doorbell mapping should work unchanged from inside the
sandbox** — no capability, no namespace escape, no relaxation.

### 3.3 ★ The same reading puts a caveat on work that is ALREADY BANKED

The row that matters for rung 2: the framebuffer is permitted read-write for a non-admin
too, so a channel's ring and USERD *should* map from inside the sandbox. But —

**MEASURED, and this is the honest boundary:** rung 2's mapping was only ever exercised by
`kayfabe-rm-ladder`, which runs as **root and unsandboxed**. It therefore took the
`osIsAdministrator()` fast path at `osapi.c:2034` and **never executed the validation code
above**. The ladder's own R10/R11 rows do run the real sandboxed isolate, but they only
issue ioctls (`Publish` = alloc + `map_gpu_va`); **no CPU mapping has ever been made from
inside the sandbox, of anything.**

So the correct statement about rung 2 is: *the mapping protocol is measured, privileged;
the unprivileged path is source-derived and untested.* That is weaker than the rung 2
report implied, and it is written here rather than left to be discovered.

### 3.4 What remains UNKNOWN

- Whether `nv_check_usermap_access_params`
  (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:2322-2327`) imposes anything further.
  Not read.
- Whether the isolate's **256 KiB read-only tmpfs root** matters at all. It should not —
  `mmap` of an already-open descriptor touches no path — but the tmpfs *size* is a real
  bound on anything that needs a file, and nothing in this path does.
- Whether a `PROT_WRITE` mapping of a range validated as `NV_PROTECT_READABLE` fails at
  `mmap` or silently downgrades. Irrelevant for the usermode window, which is read-write,
  and it is the thing to check first if a timer or master-control range is ever mapped.

---

## 4. ★★ The cacheability is DIFFERENT from the ring's, and a copy-paste gets it wrong

**SOURCE.** `map_cpu` currently hardcodes `CachePolicy::WriteCombining`, which is right for
a framebuffer object and **wrong for the doorbell**. The usermode window is a BAR0
*register* range (§3.2), so `nvidia_mmap_helper` takes the `IS_REG_OFFSET` branch and calls
`nv_encode_caching(&vma->vm_page_prot, NV_MEMORY_UNCACHED, NV_MEMORY_TYPE_REGISTERS)`
(`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574`) — **uncached**, unconditionally. The
framebuffer branch two lines below (`:575-597`) is the write-combining one, and it has its
own sub-case that maps the USERD window uncached.

★ **This would not have been caught.** `Backing::DeviceFile`'s attainable policy is `None`
by design (rung 2), so `require_attainable` cannot refuse a wrong requirement over a device
fd — the mapping would have succeeded carrying a false claim in the source, which is
exactly the failure `cache.rs` is written to prevent one layer up.

**Consequence for whoever builds rung 3:** `map_cpu` must take the [`CachePolicy`] as a
parameter rather than hardcoding one, and the doorbell call site must pass `Uncached`. That
is a one-line signature change and it should be made *before* the doorbell is written, not
after, because there is no test that can fail if it is not.

---

## 5. The fence, which has no mechanism yet

**SOURCE + already documented.** `VolatileRegion`'s own docs name this seam and say the
mechanism does not exist: stores to a write-combining mapping are **not ordered against
each other**, so a doorbell store can become visible to the device before the pushbuffer
bytes it announces, and the GPU then executes whatever was there before. The fix is a
**release fence before the doorbell store** (`sfence` on x86, `dsb st` on arm64 — NVIDIA's
own `os_flush_cpu_write_combine_buffer`).

★ Note the interaction with §4: the *doorbell* page is uncached, so the doorbell store
itself is not combined — but **the ring is write-combining**, and it is the ring's stores
that must be ordered before the doorbell. The fence belongs between them, and it is owed by
whoever writes `ring_doorbell`, not by the mapping.

---

## 6. The shape rung 3 should take, and the evidence it must produce

Sketched, not built. The point of writing it down is that the *evidence design* is the hard
part and it survives the loss of the hardware.

```text
  usermode = NV_ESC_RM_ALLOC(AMPERE_USERMODE_A) under the SUBDEVICE, no params
  (node, region) = map_cpu(usermode, 64 KiB, CachePolicy::Uncached)     <- see §4
  build a pushbuffer at ring + PUSHBUFFER_OFFSET:
      method_header_inc(subchannel 0, fifo::SEM_ADDR_LO, count 5)
      SEM_ADDR_LO/HI  = ring_va + SEMAPHORE_OFFSET
      SEM_PAYLOAD_LO/HI, SEM_EXECUTE = fifo::SEM_EXECUTE_RELEASE_32BIT
  gp_entry(ring_va + PUSHBUFFER_OFFSET, len) -> ring + GPFIFO_OFFSET
  release fence                                                          <- see §5
  USERD GP_PUT = 1
  release fence
  usermode region store_u32(USERMODE_NOTIFY_CHANNEL_PENDING, token)      <- THE DOORBELL
  poll the semaphore word
```

Every constant above already exists in `kayfabe_abi::submit`, landed and unit-tested at
rung 1. **No engine object is required** — these are host-FIFO methods executed by the
channel's own front end, which is why this is the smallest thing that can produce evidence.

**The evidence bar, which is the part not to compromise on:** the semaphore word must
transition `0 -> payload`, and `GP_GET` in USERD must advance `0 -> 1`. `GP_GET` is the
only word in the whole crate that hardware writes and we do not. A test that passes because
our own store read back proves nothing; the C's equivalent verdict line is *"SEM LANDED —
host doorbell+schedule+USERD mechanics GOOD"*.

★ Pick the payload so a false pass is impossible: **not** zero (the initial value) and not
the token (which we stored elsewhere and could alias).

---

## 7. ★ The `userdOffset[0] := 0x2000` bite — owed, and NOT yet paid

Stated plainly because it has now been deferred twice.

- **Rung 1: run, did not fire.** Correct — nothing read USERD at all.
- **Rung 2: run, did not fire.** Correct — USERD was mapped but still nothing *read* it.
- **Rung 3: NEVER REACHED.** The box went down first.

This is the C's own M5.47 root cause (`C: src/qemu/nvkvm_gpu_emul.c:9291-9299`): USERD lives
at `hUserdMemory[0] + userdOffset[0]`, so a non-zero offset makes hardware read USERD past
where our `GP_PUT` lands, the GPU sees `GP_PUT == GP_GET` forever, fetches nothing, and
reports **no error at all**. Zero utilisation and no Xid is the worst failure shape
available, which is why the bite is worth paying rather than dropping.

**It becomes observable at exactly the step in §6 where `GP_GET` must advance**, and it is
owed there. Whoever builds rung 3: set `userd_offset_0` to `0x2000`, expect the semaphore to
stay `0` and `GP_GET` to stay `0` **with no error reported anywhere**, and restore.
