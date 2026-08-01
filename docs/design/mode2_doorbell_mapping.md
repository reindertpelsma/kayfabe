# The doorbell mapping, and whether it survives the sandbox

**Status: rungs 3 and 4 are BUILT and MEASURED on hardware** — RTX 3090 (GA102) /
580.159.04 open kernel modules, 2026-07-30. §§1–6 below were written *before* the run and
are left as they were; §8 is what the hardware said, including the one place §6's analysis
was wrong. Every claim carries a label and they are used strictly:

| label | meaning |
|---|---|
| **MEASURED** | observed on real hardware, with the output quoted |
| **SOURCE** | read out of `research_clones/ogkm-580.159.04/`, cited to file:line. Not run. |
| **UNKNOWN** | nobody knows, and this file says so rather than guessing |

★ The part worth reading first is §8.3: **the prediction in §3 held**, and the failure that
actually cost the run was one nobody had predicted at all.

★★ **Chip note.** The measurements are on **GA102**, not the GA106 the C bench used. RM
ioctls, channel allocation, doorbells and CE copies are chip-independent, and nothing below
touches VBIOS or a chip-specific register. The one place the part *could* matter is named
where it arises (§8.4: `COPY0` is a graphics copy engine on **this** architecture).

---

> ★ **The token's ENCODING is a different document.** This one is about *where* the
> doorbell is and *whether the sandbox can reach it*. What the 32 bits mean —
> `NV_CTRL_VF_DOORBELL_VECTOR` 11:0 and `_RUNLIST_ID` 22:16, established against RM's own
> compiled encoder and against a real GA106 — is `doorbell_token_encoding.md` (increment
> E3, 2026-08-01). §8.4's runlist table below reads the runlist **out of the token**, which
> that document records as circular and replaces.

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

## 7. ★ The `userdOffset[0] := 0x2000` bite — **PAID, and it fired** (see §8.2)

Stated plainly because it had been deferred twice.

- **Rung 1: run, did not fire.** Correct — nothing read USERD at all.
- **Rung 2: run, did not fire.** Correct — USERD was mapped but still nothing *read* it.
- **Rung 3: RUN, AND IT FIRED**, exactly as predicted below.

This is the C's own M5.47 root cause (`C: src/qemu/nvkvm_gpu_emul.c:9291-9299`): USERD lives
at `hUserdMemory[0] + userdOffset[0]`, so a non-zero offset makes hardware read USERD past
where our `GP_PUT` lands, the GPU sees `GP_PUT == GP_GET` forever, fetches nothing, and
reports **no error at all**. Zero utilisation and no Xid is the worst failure shape
available, which is why the bite is worth paying rather than dropping.

**It becomes observable at exactly the step in §6 where `GP_GET` must advance**, and it is
owed there. Whoever builds rung 3: set `userd_offset_0` to `0x2000`, expect the semaphore to
stay `0` and `GP_GET` to stay `0` **with no error reported anywhere**, and restore.

---

## 8. ★★★ WHAT THE HARDWARE SAID — RTX 3090 (GA102) / 580.159.04 open, 2026-07-30

Rungs 3 and 4 were built and run. `kayfabe-rm-ladder --gpu 0`, exit code 0. The three new
rows, quoted verbatim:

```
★     R15 SEM LANDED      = sem 0xbeef5ea1 (want 0xbeef5ea1), GP_GET 1 -> caught GP_PUT 1
                            — the GPU consumed our ring and released our semaphore
★     R17 CE COPY         = 4096 bytes: dst[0] 0x3f0011ff -> 0xc0ffee00,
                            dst[last] 0xc0fff1ff (want 0xc0fff1ff)
                            — read back through an INDEPENDENT mapping
★     R16 sandboxed doorbell = the capability-less isolate CPU-mapped the ring, USERD and
                            the usermode BAR0 window, and rang channel 0xcafe000c token 0x4
```

### 8.1 ★★ The sandbox question, answered: the mapping SURVIVES

**MEASURED.** §3's source-derived prediction held. R16 drives the production
`VerbPlan::Doorbell` chain through the **real sandboxed isolate** — its own user namespace,
every capability dropped, `NoNewPrivs`, a 256 KiB read-only tmpfs root holding only
`/dev/nvidiactl` and `/dev/nvidia0`, exec'd from a sealed memfd — and it succeeded. That
chain makes **three** CPU mappings from inside the sandbox, covering both whitelist rows of
§3.2 in one plan:

| mapping | BAR0 row | result |
|---|---|---|
| the GPFIFO ring (`NV01_MEMORY_LOCAL_USER`) | framebuffer, *"See bug 1784955"* | **mapped** |
| USERD (`NV01_MEMORY_LOCAL_USER`) | framebuffer | **mapped** |
| the `AMPERE_USERMODE_A` window | `kfifoGetUsermodeMapInfo_HAL`, the read-write one | **mapped** |

So §3.3's honest boundary — *"no CPU mapping has ever been made from inside the sandbox, of
anything"* — is retired. No capability was restored, no namespace was escaped, nothing was
relaxed. `RmValidateMmapRequest`'s validation path really does whitelist the doorbell
window read-write for a caller with no `CAP_SYS_ADMIN`, which is the same thing that lets an
ordinary CUDA process ring its own doorbell.

⊘ **What R16 does NOT prove, stated because the ★ line could be read as more than it is:**
the sandboxed path produces **no submission evidence**. The isolate's channel is its own,
its ring is in its own address space, and the port's verb surface has no way to build a
pushbuffer through it — so R16 shows the mapping and the store succeeded, not that anything
executed. The semaphore-and-`GP_GET` evidence (R15) is from the **unsandboxed** ladder.
Closing that gap needs a plan variant that submits *and* reports, and it is not built.

### 8.2 ★★★ The `userdOffset` bite: INDUCED, WATCHED, REMOVED

**MEASURED.** With `userd_offset_0 := 0x2000` and nothing else changed:

```
ok    R13.1 channel      = 0xcafe000d, engine Ce, token 0x00000004 (runlist 0 chid 4)
ok    R13.1 schedule     = on the runlist
FAIL  R15 SEM NEVER LANDED= sem 0x00000000 (want 0xbeef5ea1), GP_GET 0 GP_PUT 1
FAIL  R17 CE COPY         = dst[0] 0x3f0011ff -> 0x3f0011ff (want 0xc0ffee00)
```

Precisely the predicted shape, and precisely why it was worth paying: **every ioctl still
returned 0.** The channel allocated, the group bound, the schedule succeeded, the token came
back, the doorbell store completed. Nothing anywhere reported an error; the only thing that
said anything was wrong was a word in device memory that did not change. Restored to `0`
and re-run green.

Two further bites were run on the same principle:

- **The doorbell store removed** (the `store_u32` replaced by `Ok(())`) → `sem 0x00000000
  GP_GET 0 GP_PUT 1`. So R15's green is *caused by the doorbell*, not something that would
  have happened anyway. The write-combining release fence protects a live seam, not a
  hypothetical one.
- **The `AMPERE_DMA_COPY_B` engine object not allocated at all** → **the copy still
  worked**. See §8.4.

### 8.3 ★★★ The failure §6 did not predict — and a wrong diagnosis, corrected by its own bite

Rung 4's first run failed: `CE_NEVER_RETIRED`. The first diagnosis was that `SET_OBJECT`
carries a **class id** (`NVC56F_SET_OBJECT_NVCLASS` is bits 15:0,
`ogkm-580: clc56f.h:68-71`) and the code was passing an object *handle*. Two things were
changed at once — the data word *and* the subchannel — and it went green.

The bite that was supposed to confirm the diagnosis **disconfirmed it**. Isolated, one
variable at a time on the same hardware:

| subchannel | `SET_OBJECT` data | result |
|---|---|---|
| 0 | the class id, correct | `GP_GET` advanced to `GP_PUT`, **semaphore never released, destination byte-for-byte unchanged** |
| 4 | the class id, correct | 4096 bytes copied, semaphore released |
| 4 | a garbage handle (`0xCAFE_000E`) | **4096 bytes copied anyway** |

So **the subchannel is what routes**, and `SET_OBJECT`'s data was not observed to matter at
all on this part. The class is still sent, because that is what the encoding says and what
UVM sends (`ogkm-580: kernel-open/nvidia-uvm/uvm_maxwell_ce.c:36`) — but the code now says
in as many words that this is an argument from source, not a measurement.

★ **The methodological point, which is the transferable part:** the green run was
attributed to the change that *looked* like a bug fix, and only running the bite separated
the two. "It went green when I changed X and Y" is not evidence about X.

### 8.4 ★★ Why subchannel 4, and the one place the chip could matter

**SOURCE + MEASURED.** `NVA06F_SUBCHANNEL_COPY_ENGINE = 4`
(`ogkm-580: kernel-open/nvidia-uvm/cla06fsubch.h:30`), and UVM's own comment
(`uvm_maxwell_ce.c:31-36`) says subchannel 4 is *"required to match CE usage on GRCE"*.
GRCE is exactly what this port gets: rung 1's `--engines` sweep measured
`NV2080_ENGINE_TYPE_COPY0` landing on **runlist 0**, the graphics runlist, because on this
architecture the first two logical copy engines *are* the graphics copy engines.

⚠ That last sentence is the chip-dependent one. It was measured on GA106 (rung 1, RTX 3060)
and again on GA102 (RTX 3090) — the same answer both times — but a part whose CE0 is not a
GRCE could behave differently on subchannel 0, and this file should not be read as claiming
otherwise.

**Also measured, and deliberately not acted on:** the copy works with **no engine object
allocated**. It is allocated anyway — the C allocates it, UVM allocates it, and a channel
with no engine context is not something to depend on being fine — but that is an argument
from those sources, not a bite that fired. A step kept for an undemonstrated reason must say
so, rather than be quietly deleted because a test did not notice it.

### 8.5 The cache-policy correction, applied before it could bite

**Applied, not measured** — and it *cannot* be measured from userspace, which is the point.
`map_cpu` no longer hardcodes `CachePolicy::WriteCombining`; it takes the policy as a
parameter, and the doorbell window passes `Uncached` (a BAR0 register range,
`ogkm-580: kernel-open/nvidia/nv-mmap.c:567-574`) while the ring and USERD pass
`WriteCombining` (framebuffer objects, `:575-597`). `Backing::DeviceFile`'s attainable
policy is `None` by design, so `require_attainable` **cannot** refuse a wrong requirement
over a device fd — there is no test in this workspace that could have failed on the old
constant, which is exactly why it had to be fixed before the doorbell was written rather
than after.

### 8.6 What rung 4 is, precisely

`ce_copy`'s `HostCe` arm forwards a **VA-addressed** copy: `LAUNCH_DMA` goes out with
`SRC_TYPE_VIRTUAL` and `DST_TYPE_VIRTUAL`, so the engine walks the isolate's own host VAS
(#14's per-`Vas` boundary) and cannot be pointed at physical memory even by a wrong address
— `kayfabe_abi::submit::ce` does not define the `_PHYSICAL` constants at all. That is the
owner's ruling implemented: *only a CE whose operands are genuinely GPGA must be emulated;
everything VA-addressed can be forwarded, because we control the mapping.*

Still refused, by name:

- **`CeExecutor::Ours`** — needs the isolate's mapping of the fabricated aperture, whose
  extent is an open owner question. Unchanged.
- **`CeSource::Constant`** — a fill is `LAUNCH_DMA` with `REMAP_ENABLE` plus the
  `SET_REMAP_*` block, which the ABI module does not transcribe. Emitting a copy from
  address 0 instead would scrub the destination with whatever is at VA 0.
- **`fb_read`** — unchanged (`NOT_ON_THIS_RUNG`).
