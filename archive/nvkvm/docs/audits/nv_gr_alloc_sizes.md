# nvkvm task #78 — GR / Compute alloc-param sizes (host-verified)

Read-only audit. Goal: identify the alloc-parameter struct and its `sizeof` for every
compute/graphics (3D) engine alloc class missing from nvkvm's guest sizing table in
`src/guest/nvkvm_main.c`.

## Key facts (all verified on the 575 open-driver host, not eyeballed)

- **Struct used by ALL compute (`*C0`) and graphics 3D (`*97`) classes:**
  `NV_GR_ALLOCATION_PARAMETERS` (defined in `sdk/nvidia/inc/nvos.h`).
  - Layout: `NvU32 version; NvU32 flags; NvU32 size; NvU32 caps;` → 4 × u32.
  - **sizeof = 16 bytes** — compiled & printed on host
    (`cc -I .../inc grsz.c` → `NV_GR_ALLOCATION_PARAMETERS=16`).
- Kernel binding confirmed in
  `src/nvidia/src/kernel/rmapi/resource_list.h`: every class below has
  `Internal Class = KernelGraphicsObject` and `Alloc Param Info = RS_OPTIONAL(NV_GR_ALLOCATION_PARAMETERS)`.
  Because it is `RS_OPTIONAL`, libcuda may pass `alloc_parms_size = 0`; the driver then
  sizes by hClass. That is exactly the path the guest sizing table must cover (the `ap_size == 0`
  fallback `switch` in `nvkvm_main.c`).

## CRITICAL bug found in nvkvm's existing class table

`src/abi/nvgpu.h` currently has WRONG hex codes for the compute classes:

```
#define TURING_COMPUTE_A   0x0000C4B1U   // WRONG — 0xC4B1 is not a class; real = 0xC5C0
#define AMPERE_COMPUTE_A   0x0000C6B1U   // WRONG — real = 0xC6C0
#define HOPPER_COMPUTE_A   0x0000CBB1U   // WRONG — real = 0xCBC0
```

The `*B1` suffix is bogus (mix-up of the `*97` graphics suffix and `*C0` compute suffix).
The real SDK values (from `sdk/.../class/cl*.h`) are listed below. These macros must be
fixed before the classes can ever match in the size `switch`. They are also currently
UNUSED in any `switch`, so even with correct values nothing sizes them yet.

## Class table — host-verified

| Class name              | hClass (real SDK) | alloc-param struct           | sizeof | present in nvkvm? |
|-------------------------|-------------------|------------------------------|--------|-------------------|
| **Compute (`*C0`)**     |                   |                              |        |                   |
| VOLTA_COMPUTE_A         | 0xC3C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| VOLTA_COMPUTE_B         | 0xC4C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| TURING_COMPUTE_A        | 0xC5C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (wrong macro 0xC4B1) |
| AMPERE_COMPUTE_A        | 0xC6C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (wrong macro 0xC6B1) |
| AMPERE_COMPUTE_B        | 0xC7C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (no macro)     |
| ADA_COMPUTE_A           | 0xC9C0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (no macro)     |
| HOPPER_COMPUTE_A        | 0xCBC0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (wrong macro 0xCBB1) |
| BLACKWELL_COMPUTE_A     | 0xCDC0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (no macro)     |
| BLACKWELL_COMPUTE_B     | 0xCEC0            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no (no macro)     |
| **Graphics 3D (`*97`)** |                   |                              |        |                   |
| VOLTA_A                 | 0xC397            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| TURING_A                | 0xC597            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| AMPERE_A                | 0xC697            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| AMPERE_B                | 0xC797            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| ADA_A                   | 0xC997            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| HOPPER_A                | 0xCB97            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| BLACKWELL_A             | 0xCD97            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |
| BLACKWELL_B             | 0xCE97            | NV_GR_ALLOCATION_PARAMETERS  | 16     | no                |

All of the above are **missing** from the guest sizing `switch` in
`src/guest/nvkvm_main.c` (two copies: nvos21 path ~line 1063, nvos64 path ~line 1139).
None are referenced by any `case` in either `switch`.

## What is already present in nvkvm's size table (for contrast)

Both `switch` blocks in `nvkvm_main.c` currently size only:
NV01_DEVICE_0, NV20_SUBDEVICE_0, RM_USER_SHARED_DATA, FERMI_VASPACE_A,
NV50_MEMORY_VIRTUAL / NV01_MEMORY_LOCAL_USER / NV01_MEMORY_SYSTEM,
KEPLER_CHANNEL_GROUP_A, FERMI_CONTEXT_SHARE_A,
TURING/AMPERE/HOPPER_CHANNEL_GPFIFO_A, NV01_EVENT_OS_EVENT,
VOLTA/TURING/AMPERE_A/AMPERE_B/HOPPER/BLACKWELL_DMA_COPY_A, GT200_DEBUGGER.
No compute or 3D graphics object class is handled.

## QEMU sanitizer table — NOT affected

`src/qemu/nvkvm_isolate_handlers.c` has **no** per-class alloc-param size table. The QEMU
side forwards the aux buffer using `req->aux_size`, which the **guest** sets. It only peeks
at `hClass` (offset 12) for the post-alloc SHARE/dup decision. So the GR fix is **guest-only**:
add the classes above to the two `switch` statements in `nvkvm_main.c` and fix the bogus
`*_COMPUTE_A` macros in `src/abi/nvgpu.h` (add the `_B` / ADA / BLACKWELL macros too).

## Suggested guest-side change (informational, not applied)

Add to both `switch` blocks:
```c
case TURING_COMPUTE_A:  case AMPERE_COMPUTE_A:  case AMPERE_COMPUTE_B:
case ADA_COMPUTE_A:     case HOPPER_COMPUTE_A:  case BLACKWELL_COMPUTE_A:
case BLACKWELL_COMPUTE_B: case VOLTA_COMPUTE_A: case VOLTA_COMPUTE_B:
case TURING_A: case AMPERE_A: case AMPERE_B: case ADA_A:
case HOPPER_A: case BLACKWELL_A: case BLACKWELL_B: case VOLTA_A:
    ap_size = sizeof(struct nv_gr_allocation_parameters); /* 16 */
    break;
```
(nvkvm must add a `struct nv_gr_allocation_parameters { u32 version, flags, size, caps; }`
to `src/abi/nvgpu.h` — there is no existing 16-byte 4×u32 struct named for GR.)

### Host verification command used
```
cc -I /root/open-gpu-kernel-modules/src/common/sdk/nvidia/inc grsz.c -o grsz
./grsz   →  NV_GR_ALLOCATION_PARAMETERS=16
```
