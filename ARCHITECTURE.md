# kayfabe architecture — v3 on one page

**STATUS: LIVE, 2026-09-26, at `master` `74dc3113`.** This page is a map. The design authority is
[`docs/design/THE_ARCHITECTURE_v3.md`](docs/design/THE_ARCHITECTURE_v3.md) (its argued-from-scratch
statement is [`docs/design/THE_DESIGN.md`](docs/design/THE_DESIGN.md)), and the build
decisions are in [`docs/design/V3_BUILD.md`](docs/design/V3_BUILD.md). Where this page and those
documents disagree, they win. ⊘ The pre-v3 map (the hexagonal core, `kayfabe-core`/`-fwd`/`-rt`,
and the invariant catalog that old source comments cite as *"`ARCHITECTURE.md` invariant N"*) is
archived at [`docs/archive/ARCHITECTURE_pre_v3.md`](docs/archive/ARCHITECTURE_pre_v3.md).

## The shape

```
 guest (untrusted: userspace AND guest root)
   stock NVIDIA driver 580.159.04 ── GSP RPCs, page tables, doorbells ──┐
                                                                        │ BAR0 writes only
 ┌─────────────── one host process: QEMU + the kayfabe core ────────────▼───────────────┐
 │ kf3-gpu device (qemu/hw/misc/kf3 + kf-qemu)                                           │
 │   vCPU trap path (kf-trap): BAR0 writes only; token words, rung bitmap, wake word     │
 │     ├─ privileged ring ─► 1 register drainer: GSP RPC kicks, interrupt tree, TLB ops  │
 │     │                      └─ faked GSP (kf-gsp) + emulated RM (kf-rm, kf-chip, kf-abi)│
 │     └─ doorbell plane  ─► N workers: claim a rung token, service its channel (kf-chan)│
 │   memory (kf-mem + kf-cuda): one vidmem store; a CUDA walk kernel diffs the guest's   │
 │     live page tables against what the host confirmed it placed (commit-on-ack)        │
 │   host RM verbs (kf-host): one unprivileged RM session, every verb AUTHORED           │
 └───────────────────────────────────────────┬───────────────────────────────────────────┘
                                             ▼
                          host GPU via /dev/nvidia* (the stock host driver)
```

- **One process.** The VMM with the kayfabe core linked in. There are no isolate children, no IPC
  plane, and no sandbox of kayfabe's own. To sandbox, confine the whole hypervisor process
  (`THE_ARCHITECTURE_v3.md` §1).
- **Only BAR0, only writes, are trapped.** BAR1 and BAR2 are never trapped (§2). The vCPU never
  blocks on host work: privileged writes go into one ordered ring, drained by one thread
  (§2.3). Doorbells set a bit in a token table, which workers scan (§2.2).
- **Control plane: authored, never forwarded.** The guest's RM talks to a faked GSP. Kayfabe
  answers from (a) facts that it reads from the host GPU through **unprivileged** RM controls,
  (b) family rules and register offsets generated from NVIDIA's open-driver headers, or
  (c) values that it fabricates so that RM's own code accepts them. Every field records which
  of the three it came from (`kf-chip`, `V3_BUILD.md` crate map). Host actions are issued as
  kayfabe's own RM verbs with kayfabe's own flags. Nothing the guest sends is replayed to the
  host.

## Channels — three kinds

From [`docs/design/the_three_channel_kinds.md`](docs/design/the_three_channel_kinds.md) and
[`THE_TRANSLATED_PLANE.md`](docs/design/THE_TRANSLATED_PLANE.md):

| kind | who | what kayfabe does | completion |
|---|---|---|---|
| **Passthrough** | guest **user** channels (CUDA, Vulkan, NVENC/NVDEC…) | births a host **twin** channel in the host channel group that matches the guest's (one host TSG per guest TSG); the guest's pushbuffer runs on the real engine unparsed | the GPU writes it; kayfabe does nothing |
| **Translated** | guest **kernel** channels (RM's CeUtils scrubber, UVM's CE) | rewrites the physically addressed CE work onto kayfabe's own window and rings a host ring; the real engine executes it | the GPU writes the semaphore that was forwarded |
| **Emulated** | channels with no GPU work behind them | executes in the VMM | forged at the end of kayfabe's own call, because no GPU work was involved |

There is **no CPU executor for GPU work** (`kf-chan`: *"Never a CPU executor"*). Host events reach
the guest as host event → eventfd → MSI.

## Memory and addresses

- **One store.** Guest vidmem is one host allocation, and its size is a hypervisor command-line
  parameter (`fb-mb=`). A guest FB offset *is* the offset into that allocation. Guest sysmem is
  the hypervisor's shared memfd (§3).
- **Mirror, never adopt** (§4.1). The host twin's VA space is populated with kayfabe's own host
  mappings. Kayfabe never points the host at the guest's tables.
- **The diff** (§4.2, owner ruling 2026-09-25). A PTX walk kernel (`cuda/walk`, `kf-cuda`) reads
  the guest's live page tables *in place* in vidmem. It emits only a diff against the placements
  that the host confirmed it made. That record is kept in vidmem per VA-space object. The host
  applies the diff as batched maps and unmaps, with the TLB-defer flag and one invalidate, and
  acknowledges run by run (commit-on-ack). A refused map stays a difference and is retried.
  `kf_cuda::diffmodel` is the spec; `kf-gate9` holds the GPU to it. VA-contiguous guest-RAM runs
  are mapped as one OS descriptor ([`V3_BATCHED_MAP.md`](docs/design/V3_BATCHED_MAP.md)).
- **A VA space is an object; its PDB is an attribute, per GPU**, and it may be absent (§4.3).

## Crates

The v3 crates (`crates/kf-*`, descriptions condensed from their own `Cargo.toml`):

| crate | job |
|---|---|
| `kf-trap` | vCPU trap path and doorbell plane: token words, rung bitmap, wake word, privileged ring. Lock-free, allocation-free, no OS call |
| `kf-core` | plane composition: trap → token plane → workers + one register drainer; host behind `HostOps` |
| `kf-gsp` | the faked GSP: falcon boot FSM, message queues, RPC framing |
| `kf-rm` | the emulated GSP-side RM: RPC answers (static info, init tables), object graph, host-fact controls |
| `kf-chip` | the chip model on the compatibility axes: one family row per generation, register offsets generated from ogkm, per-die facts read from the host |
| `kf-abi` | the generated NVIDIA ABI (classes, controls, RPC layouts), version-keyed. The only home of NVIDIA `#[repr(C)]` layouts |
| `kf-host` | host RM verbs in-process: one RM session on the host GPU, verbs authored |
| `kf-mem` | the store, the VA manager step, batched apply with one TLB invalidate |
| `kf-cuda` | the walk kernel, in-process: a `dlopen`'d CUDA driver-API binding and the committed PTX |
| `kf-chan` | channels: passthrough birth, Translated execution, VMM-executed emulated channels, completions as host events |
| `kf-qemu` + `qemu/hw/misc/kf3/` | the `kf3-gpu` device: realize, the plane, the register drainer, the FFI to a small C QOM device |
| `kf-harness` | the GPU gates `kf-gate1`…`9`: play the guest against the real planes, with no QEMU |
| `kf-crec`, `kf-trace` | C-trace differential (the `cap1`/`cap1b` replays). Oracle only, never linked into the product |
| `kf-arch`, `kf-linux-raw`, `kf-util` | vocabulary and the `Arch` traits; the audited raw-OS adapter; generic utilities |

`crates/kayfabe-*` is the **frozen pre-v3 tree**. It is kept because `kayfabe-rm-ladder`, the
30-arm raw-client grader behind the thin-guest suite and the bare-metal baseline, depends on it.
No `kf-*` crate depends on a `kayfabe-*` crate.

## Rules that shape the code

- **Hostile guest, two boundaries.** The guest is untrusted, and so is the boundary *inside* the
  guest: unprivileged guest userspace can write any doorbell token. No check at the trap can
  establish identity. The design limits what such a write can *reach* (`THE_ARCHITECTURE_v3.md`
  §2.1).
- **Derive, do not hardcode** (§7). Prefer host userspace queries, then computed values, then
  Rust generated from the ogkm headers with a real parser. Keep a small set of per-family
  constants. Never keep a captured per-die table. *All GPU families are first-class; GA10x is not
  primary.*
- **No blocking work on a vCPU, and none under a lock that a vCPU takes**
  (`THE_CONSTRAINTS.md`).
- **Never copied into v3:** CPU reads of guest page tables, CPU-executed engine work, address
  tables and joins, isolates and the IPC plane, publication epochs and dirty gates, per-page BAR
  traps, and snapshots of the guest's tables (`V3_BUILD.md`, *Rules*).
- **Unsafe code** is allowed only in `kf-linux-raw`, `kf-qemu` and `kf-cuda`, and only in files
  named `*_unsafe.rs`.

## Verification

| gate | what it proves | where |
|---|---|---|
| `kf-*` crate tests | logic, generated ABI against the oracles, C-trace replays (`cap1`, `cap1b`) | `cargo test -p kf-…`, GPU-free |
| `kf-gate1`…`9` | the real planes on a real GPU, with no QEMU: a real CE on kayfabe's own ring completing through a host event (1); the host VAS matching the guest's tables, VER2 and VER3 formats (2, 7); Translated kernel CE (3); doorbells driving workers (4); passthrough on CE and GR (5, 6); invalidate completes only after the host map is committed (8); the walk kernel's diff protocol (9) | `scripts/bench/v3_gates.sh` |
| thin guest, 30 arms | stock driver boot plus the raw client, one boot per arm, a hard time budget; bare metal is the baseline | `scripts/fastguest/fast_suite.sh` (`KF_DEVICE=kf3`) |
| fat guest lanes | CUDA ladder (`cup3`, `cup8`), app matrix, LLM parity, graphics, video | `scripts/bench/boot_capture.sh` + the lane scripts |

A bench claim carries the source revision it was measured at. `build_kf3.sh` installs each
revision's binary under `kf3-bins/<rev>/`, and the harnesses run that binary.
