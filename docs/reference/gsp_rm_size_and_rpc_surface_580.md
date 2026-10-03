# GSP-RM size and the GSP RPC surface, driver 580.159.04

**STATUS: REFERENCE, 2026-10-02.** It answers the owner's question of how much of RM lives on the
GSP, and what the RPC interface between the two halves really is. Everything was counted on the dev
host, from `NVIDIA-Linux-x86_64-580.159.04.run` (extracted with `-x`) and from the open source tree at
the same tag (`research_clones/ogkm-580.159.04` in the nvkvm tree). Nothing was run on a GPU.

## 1. Code size

Instruction counts are from `llvm-objdump` 21 (`-d --no-show-raw-insn`), counted on 2026-10-02 against
driver 580.159.04.

| image | what it is | instructions | function symbols |
|---|---|---|---|
| `kernel-open/nvidia/nv-kernel.o_binary` | the open kernel RM, the half that ships in open-gpu-kernel-modules | 0.92 M (x86-64) | 16 269 |
| `kernel/nvidia/nv-kernel.o_binary` | the closed full RM, which can also drive a GPU without GSP | 3.67 M (x86-64) | 59 476 |
| `firmware/gsp_tu10x.bin`: `.fwimage`, then `.section_task_rm_elf_text_instance` | GSP-RM for TU10x, TU11x and GA100 | 4.15 M (RISC-V) | — |
| `firmware/gsp_ga10x.bin` | a flat 75 MB image, signed for GA10x, AD10x, GH100, GB10x/GB10y, GB20x/GB20y and their CC variants | not split | — |

- **The GSP RM task's text is 16.6 MB.** It decodes as ordinary compiled code; the most common
  mnemonics are `ld`, `addi`, `sd`, `li`, `lui`, `mv` and `jalr`. Its data is a further 11.7 MB.
- **Byte sizes:**
  - closed RM: 15.19 MB of code and 82.49 MB of read-only data;
  - open RM: 3.85 MB of code and 8.26 MB of read-only data.
- **Source lines (open tree):** `src/nvidia` is 1.37 M lines, of which 0.75 M are NVOC-generated
  `g_*` files, so about 0.62 M are hand-written.
- **Reading:**
  - GSP-RM is about the size of the closed full RM, compiled for RISC-V without the OS layer.
  - ogkm's RM is roughly a quarter of it.
  - Most of the rest is per-chip hardware management: engine and graphics init, scheduling, MMU and
    fault buffers, interrupts, power and clocks, display hardware, and confidential compute on
    Hopper and later.
- **Caveats:**
  - The closed RM also carries pre-Turing chips, so the per-chip comparison is approximate.
  - RISC-V usually needs somewhat more instructions than x86-64 for the same source.

## 2. The RPC table

`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc_global_enums.h` has 229 function slots (0–228) and 36
GSP→CPU events. The table is shared with vGPU, where a guest RM talks to the host plugin, so most
slots never travel to GSP. The buckets below were sorted by name pattern, so the topic split is
approximate.

| slots | what they are |
|---|---|
| 27 | deprecated or reserved |
| 115 | one dedicated RPC per RM control, used on the vGPU path. On GSP the same controls ride inside `GSP_RM_CONTROL`. |
| 27 | the vGPU guest's object and memory API (`ALLOC_ROOT`, `ALLOC_MEMORY`, `MAP_MEMORY`, …) |
| 34 | boot information, display, page tables, power, suspend, events and others |
| 10 | the vGPU UVM paging channel (7) and simulator escapes (3) |
| 16 | the functions kayfabe's fake GSP serves (`crates/kf-gsp/src/rpc.rs`, `RpcFunction`) |

The 16 served functions:

- **boot and teardown:** guest system info and its extension, `GSP_SET_SYSTEM_INFO`, `SET_REGISTRY`,
  `GET_GSP_STATIC_INFO`, the trace crash buffer, `UPDATE_BAR_PDE`, `UNLOADING_GUEST_DRIVER`;
- **the RM API carriers:** `GSP_RM_ALLOC`, `GSP_RM_CONTROL`, `FREE`, `DUP_OBJECT`;
- **UVM:** set and unset page directory;
- **plumbing:** continuation records and the ECC notifier acknowledgement.

Anything else is refused by name. kayfabe sends three event kinds back: init done, event notifications
and channel errors.

⇒ **The real interface is the controls inside `GSP_RM_CONTROL`.** Those include 245
`NV2080_CTRL_CMD_INTERNAL_*` controls (`ogkm-580: src/common/sdk`), most of them kernel RM asking the
GSP side for facts and actions.
