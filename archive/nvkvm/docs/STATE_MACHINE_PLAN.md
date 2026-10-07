# State-machine refactor — guest-emulated lifecycle, QEMU realizes

Drafted 2026-05-28 in response to the UVM `EBADFD` mmap bug and a
broader push to lock down the QEMU↔guest ioctl surface.

## 1. Motivation

Today's forwarding model:

* The guest module receives an ioctl from libcuda, copies the params
  into shared memory, sends them through virtio to QEMU.
* QEMU either handles it directly (rare, frontend.c) or pushes it to
  the isolate (the stub process) for execution.
* For mmap: the guest module forwards a request to QEMU, which asks
  the isolate to `mmap(2)` an fd it received via SCM_RIGHTS.

Two structural problems with this:

1. **Raw fd surface in the guest kernel** — for UVM, the guest module
   shovels arbitrary `cmd` numbers down to a kernel fd that the
   isolate owns.  The kernel enforces per-fd typing (e.g.
   `uvm_mmap` requires the fd to be `UVM_FD_VA_SPACE`-typed via
   `UVM_INITIALIZE`).  Our pool of pre-opened UVM fds in the stub
   typically has only one VA-space-initialized fd; subsequent guest
   UVM opens get mapped to uninitialized pool slots and any `mmap()`
   on those returns `-EBADFD`.

2. **Limited validation** — QEMU passes parameter blobs through
   unexamined.  We can't enforce "this guest may only allocate
   semaphore pools of size N at GPA X" because we don't know what
   the guest is trying to do at the abstract level; we only see
   opaque ioctl args.

User-stated principle:

> Raw passthrough is only allowed for the isolate.  Anything QEMU
> executes itself must go through the state-machine model.

Eventually that applies to **every** guest-originated ioctl and
syscall.  We migrate piece by piece, UVM first.

## 2. Architectural principle

Two execution surfaces — **QEMU is privileged, the stub is sandboxed**.
That asymmetry drives every category boundary in this plan.

* **Isolate execution** — the stub process.  Raw ioctl passthrough is
  acceptable because the stub is sandboxed (pClient/nvfp identity
  isolation by the kernel), and any unsafe behavior is contained
  within one stub per session.  All NV_ESC_RM_* (RM) cmds run here.
  When in doubt, **prefer the stub** — it's the lower-trust execution
  context and is also the better long-term compatibility target
  (new kernel cmds added in future driver releases will Just Work
  via the stub's passthrough without QEMU changes).

* **QEMU execution** — privileged.  Owns realization (mmap, GPA
  install, KVM region wiring) and the session-policy ground truth.
  **No raw ioctl forwarding here.**  Every cmd QEMU executes is
  privileged and:
    * must be on an explicit allowlist,
    * must have every argument validated against per-session policy,
    * **must have the param size strictly checked against the known
      struct size for that cmd** — if the guest sends more bytes,
      they are truncated; if fewer, the cmd is rejected.  QEMU
      builds its own kernel-bound buffer of the exact known size
      and copies in only the validated fields.  Whatever bytes the
      guest tried to smuggle past the documented struct end never
      reach the host kernel.
    * must have every pointer translated (GVA→GPA→host VA) by QEMU
      itself, never trusted from the guest,
    * unknown cmds are **denied**, not forwarded.

  The contract: if a feature can be implemented by stub-passthrough,
  it MUST be implemented that way — QEMU only sees the high-level
  intent.  The realize messages described in §5 are the QEMU-facing
  surface; their schemas are exhaustive, every field is bound to a
  fixed kernel struct layout, and any non-conforming request is
  rejected.

The guest module becomes a state machine that:

* Tracks per-fd config (UVM init flags, registered GPUs, range
  groups, …).
* Returns canned NV_OK responses to "pure-config" ioctls without ever
  going to the host kernel.
* At the moment of mmap or any operation that genuinely needs kernel
  work, builds a single `REALIZE_*` message carrying the full,
  validated end-state intent.

QEMU validates the realize request against per-session policy, then
either:

* asks the isolate to do the kernel work in a freshly-minted fd whose
  whole lifecycle QEMU controls, or
* synthesizes the work itself (KVM region installs, GPA allocation).

## 3. Categorization framework

Every host-touching guest cmd / syscall lands in exactly one of:

* `PURE_CONFIG` — sets state inside the kernel fd, no side effects
  visible outside that fd.  Examples: `UVM_INITIALIZE`,
  `UVM_PAGEABLE_MEM_ACCESS`, `UVM_ENABLE_PEER_ACCESS`,
  `UVM_DISABLE_PEER_ACCESS`, `UVM_SET_PREFERRED_LOCATION` (when no
  page work involved).
  → Guest module **records** into the per-fd state.  Returns
    `NV_OK`.  No host call.

* `STATE_REGISTRATION` — registers an opaque kernel object (GPU,
  VAS, channel) by handle.  No memory backing yet.  Examples:
  `UVM_REGISTER_GPU`, `UVM_REGISTER_GPU_VASPACE`,
  `UVM_REGISTER_CHANNEL`, `UVM_CREATE_RANGE_GROUP`.
  → Guest module records (handle, params).  Defers the kernel call
    until the first realize.  When realize happens, all
    accumulated registrations are replayed in QEMU's controlled fd.

* `MAPPING_INTENT` — declares that a VA range should later be
  backed by something.  Examples: `UVM_ALLOC_SEMAPHORE_POOL`,
  `UVM_MAP_EXTERNAL_ALLOCATION`, `UVM_CREATE_EXTERNAL_RANGE`.
  → Guest module records the (base, length, attributes) tuple
    against the fd.  Returns NV_OK.

* `MMAP_REALIZE` — the actual `mmap(2)` syscall on a UVM-class fd.
  This is the trigger point.  Guest module:
    1. Looks up the most-specific mapping-intent whose (base, length)
       matches the requested vma.
    2. Builds a `REALIZE_UVM_MAPPING` message carrying:
        * the fd's accumulated `uvm_state` (init flags, GPU set,
          VAS set, range groups),
        * the matched mapping intent (and which "mode" — sem pool /
          external / pageable),
        * `gva`, `length`, `gpa_hint` (or 0 to let QEMU pick).
    3. QEMU spawns a fresh stub UVM fd, runs the whole init→register
       →mapping-intent sequence on it, runs `mmap(2)`, installs the
       resulting host VA as a KVM region, returns GPA.
    4. Guest module `remap_pfn_range`'s `gva → gpa` for libcuda.

* `SIDE_EFFECT` — does real work on already-realized state.
  Examples: `UVM_MIGRATE`, `UVM_POPULATE_PAGEABLE`,
  `UVM_PREFETCH`, `UVM_MAP_DYNAMIC_PARALLELISM_REGION`.
  → Routed via the (now-existing) per-session realized UVM fd in
    the isolate.  Still validated against the recorded state in
    the guest module.

* `TEARDOWN` — `UVM_FREE`, `UVM_DEINITIALIZE`,
  `UVM_UNREGISTER_*`, plus implicit teardown on fd close.
  → Guest module clears its tracked state, asks QEMU to release the
    realized fd and KVM regions.

* `DENY` — never expose.  Examples: `UVM_TOOLS_*` (debugging
  ioctls), `UVM_TEST_*`, any test-only or admin-only cmd.

For RM (`NV_ESC_RM_*`), all cmds remain in the "isolate raw
passthrough" category as before — that's the *contract* for the
isolate.

## 4. Catalog (first pass — UVM only; RM stays in isolate-passthrough)

| UVM cmd (raw NR) | Name | Category | Notes |
|---|---|---|---|
|  1 | `UVM_INITIALIZE` | PURE_CONFIG | record `flags` per-fd |
|  2 | `UVM_DEINITIALIZE` | TEARDOWN | clear state |
| 17 | `UVM_CREATE_RANGE_GROUP` | STATE_REGISTRATION | record group id |
| 18 | `UVM_DESTROY_RANGE_GROUP` | TEARDOWN |  |
| 19 | `UVM_REGISTER_GPU` | STATE_REGISTRATION | record uuid + flags |
| 1a | `UVM_UNREGISTER_GPU` | TEARDOWN |  |
| 1b | `UVM_REGISTER_GPU_VASPACE` | STATE_REGISTRATION | record rmCtrl handle |
| 1c | `UVM_UNREGISTER_CHANNEL` | TEARDOWN |  |
| 1f | `UVM_REGISTER_CHANNEL` | STATE_REGISTRATION |  |
| 22 | `UVM_FREE` | TEARDOWN |  |
| 23 | `UVM_CREATE_EXTERNAL_RANGE` | MAPPING_INTENT | (base, length) |
| 24 | `UVM_MAP_EXTERNAL_ALLOCATION` | MAPPING_INTENT | needs RM handle |
| 27 | `UVM_PAGEABLE_MEM_ACCESS` | PURE_CONFIG |  |
| 28 | `UVM_PAGEABLE_MEM_ACCESS_ON_GPU` | PURE_CONFIG |  |
| 29 | `UVM_ENABLE_PEER_ACCESS` | STATE_REGISTRATION |  |
| 2a | `UVM_DISABLE_PEER_ACCESS` | TEARDOWN |  |
| 2b | `UVM_SET_RANGE_GROUP` | STATE_REGISTRATION | (range → group) |
| 2c–34 | `UVM_SET_PREFERRED_LOCATION` etc. | PURE_CONFIG / SIDE_EFFECT | TBD by audit |
| 38 | `UVM_MIGRATE` | SIDE_EFFECT | via realized fd |
| 3c | `UVM_VALIDATE_VA_RANGE` | SIDE_EFFECT | via realized fd |
| 44 | `UVM_ALLOC_SEMAPHORE_POOL` | MAPPING_INTENT | sem-pool kind, (base, length) |
| 47 | `UVM_POPULATE_PAGEABLE` | SIDE_EFFECT | via realized fd, post-mmap |
| 48 | `UVM_VALIDATE_VA_RANGE` (alt) | SIDE_EFFECT |  |
| 56–62 | `UVM_TOOLS_*` | DENY |  |

(The boundary between SIDE_EFFECT vs PURE_CONFIG for some cmds will
need a per-cmd code read.  Track open questions in
`docs/STATE_MACHINE_PLAN_OPEN.md` as we go.)

After UVM is migrated, the same exercise repeats for:

* `nvidia0` BAR mappings (currently piggybacking on the generic
  isolate-mmap path) — same MAPPING_INTENT / MMAP_REALIZE shape.
* `nvidiactl` info-page mmaps.
* Any future fd type.

## 5. ABI — REALIZE_UVM_MAPPING and friends

New virtio cmds, with the matching response shapes.  All values
little-endian.  All structs are 8-byte aligned.

### Common header

```c
struct nvkvm_realize_hdr {
    __le32 type;           /* NVKVM_REQ_REALIZE_UVM_MAPPING etc. */
    __le32 txn_id;
    __le32 isolate_id;     /* which session (= stub) */
    __le32 fd_handle_id;   /* the guest's nvkvm_fd_ctx handle */
};
```

### `NVKVM_REQ_REALIZE_UVM_MAPPING`

Carried payload (in a separate shm slot):

```c
struct nvkvm_uvm_state {
    __le64 init_flags;                  /* recorded UVM_INITIALIZE flags */
    __le32 n_gpus_registered;
    __le32 n_va_spaces_registered;
    __le32 n_range_groups;
    __le32 _pad0;

    /* arrays: each entry is the params the guest recorded */
    struct uvm_gpu_register_entry {
        __u8   gpu_uuid[16];
        __le32 flags;
        __le32 _pad;
    } gpus[NVKVM_UVM_MAX_REG_GPUS];

    struct uvm_va_space_register_entry {
        __u8   gpu_uuid[16];
        __le32 rm_ctrl_fd_handle;
        __le32 _pad;
    } va_spaces[NVKVM_UVM_MAX_VA_SPACES];

    struct uvm_range_group_entry {
        __le64 range_group_id;
    } range_groups[NVKVM_UVM_MAX_RANGE_GROUPS];
};

struct nvkvm_realize_uvm_mapping_req {
    struct nvkvm_realize_hdr hdr;
    __le32 mode;                  /* enum below */
    __le32 _pad0;
    __le64 gva;                   /* requested guest VA (MAP_FIXED) */
    __le64 length;
    __le64 offset_hint;           /* libcuda's mmap offset (sometimes
                                     equals gva; sometimes a handle) */
    __le32 state_shm_slot;        /* slot holding nvkvm_uvm_state */
    __le32 intent_shm_slot;       /* slot holding the matched intent */
    /* mode-specific intent (in intent_shm_slot):
     *   MODE_SEM_POOL → struct uvm_sem_pool_intent
     *   MODE_EXTERNAL → struct uvm_external_intent
     *   MODE_CREATE_RANGE → struct uvm_create_range_intent
     */
};

enum {
    NVKVM_UVM_REALIZE_MODE_SEM_POOL    = 1,
    NVKVM_UVM_REALIZE_MODE_EXTERNAL    = 2,
    NVKVM_UVM_REALIZE_MODE_CREATE_RANGE = 3,
};

struct uvm_sem_pool_intent {
    __le64 base;
    __le64 length;
    __le64 gpu_attributes_count;
    struct uvm_gpu_mapping_attributes per_gpu[UVM_MAX_GPUS_V2];
};
```

Response:

```c
struct nvkvm_realize_uvm_mapping_resp {
    __le32 status;     /* 0 on success, -errno otherwise */
    __le32 rm_status;  /* the kernel's NV_STATUS from the realize */
    __le64 gpa;        /* allocated GPA the guest module remaps to */
    __le64 length;     /* echo */
};
```

Guest module on response:
* `remap_pfn_range(vma, gva, gpa, length)` — exactly the same as
  today's path, just with QEMU computing the GPA wholesale.
* Records the (gva, gpa, fd_handle_id, realize_token) for teardown.

QEMU on receiving a realize:
1. Validate `nvkvm_uvm_state` and the intent against per-session
   policy (size caps, gpu uuid in registered list, …).
2. Allocate a GPA range in the session's GPA window.
3. Send a single batched ISOLATE_CMD to the stub: `REALIZE_UVM_FD`
   carrying state + intent + chosen host VA.
4. Stub: open `/dev/nvidia-uvm`, run UVM_INITIALIZE with recorded
   flags, run each REGISTER_GPU / REGISTER_GPU_VASPACE /
   CREATE_RANGE_GROUP, run the intent (e.g. UVM_ALLOC_SEMAPHORE_POOL),
   `mmap(2)` the result at the chosen host VA, return success +
   the host VA + rmStatus.
5. QEMU installs the host VA as a KVM region at the allocated GPA.
6. QEMU responds to the guest module with the GPA.

The stub's per-session UVM fd is **owned by this realize call** and
kept alive for the lifetime of the mapping (so subsequent SIDE_EFFECT
ioctls can be routed to it).  No more shared pool.

### `NVKVM_REQ_REALIZE_TEARDOWN`

```c
struct nvkvm_realize_teardown_req {
    struct nvkvm_realize_hdr hdr;
    __le64 realize_token;   /* opaque, from previous realize resp */
};
```

QEMU: tells stub to release the UVM fd associated with this token,
uninstalls KVM regions, frees the GPA range.

### `NVKVM_REQ_SIDE_EFFECT_UVM`

```c
struct nvkvm_side_effect_uvm_req {
    struct nvkvm_realize_hdr hdr;
    __le64 realize_token;   /* must match an alive realize */
    __le32 uvm_cmd;         /* raw NR — only an allowlist matches */
    __le32 _pad;
    __le32 param_shm_slot;
    __le32 param_size;
};
```

QEMU validates `uvm_cmd` against an allowlist (POPULATE_PAGEABLE,
MIGRATE, VALIDATE_VA_RANGE…), forwards to the stub's per-realize
fd.  Stub runs the raw ioctl — still "isolate execution" — but only
on a fd whose lifecycle QEMU controls.

## 6. Guest module state — `struct nvkvm_uvm_fd_state`

Lives inside `struct nvkvm_fd_ctx` (only populated when the fd is a
UVM fd).

```c
struct nvkvm_uvm_fd_state {
    /* Set by UVM_INITIALIZE; absent until then. */
    bool                 initialized;
    u64                  init_flags;

    /* Set by UVM_REGISTER_GPU. */
    struct list_head     registered_gpus;     /* uvm_gpu_register_entry */
    /* Set by UVM_REGISTER_GPU_VASPACE. */
    struct list_head     registered_va_spaces;
    struct list_head     registered_channels;
    struct list_head     range_groups;

    /* Pending mapping intents — matched at mmap time. */
    struct list_head     intents;             /* (base, length, mode, params) */

    /* Live realizations — set when REALIZE_UVM_MAPPING returns OK. */
    struct list_head     realizations;        /* realize_token + (gva, gpa, length) */
};
```

Locking: a single per-fd mutex.  No contention with other fds.  No
contention with the rest of the guest module (no global UVM table).

## 7. Migration order

Step-by-step, each step independently shippable and verifiable.
**Each step ends with the full test passing at the milestone level
the step's target.**

* **Step A: scaffolding.**  Add `nvkvm_uvm_fd_state` struct.  Detect
  UVM fds in `nvkvm_open` (via the device-class info already in
  scope).  Allocate the state struct.  No behavior change yet.

* **Step B: PURE_CONFIG recording (additive).**  Record
  UVM_INITIALIZE flags into ctx->uvm_state.  But **also keep
  forwarding** the ioctl so the kernel-side fd still becomes
  VA_SPACE-typed.  The short-circuit lands in Step E along with the
  realize call.  Reason: short-circuiting UVM_INITIALIZE before E
  causes the kernel fd to stay untyped, and the next forwarded UVM
  cmd (REGISTER_GPU) fails — regressing the test until Step E.
  Recording is additive at Step B; replacement is wholesale at E.
  Same applies to other PURE_CONFIG cmds whose recording happens
  in Steps C/D — all forward as before until E.

* **Step C: STATE_REGISTRATION recording.**  Intercept REGISTER_GPU,
  REGISTER_GPU_VASPACE, REGISTER_CHANNEL, CREATE_RANGE_GROUP.
  Record into state.  Return `NV_OK` without forwarding.  **At this
  point the host kernel has zero UVM state for this session — only
  the guest module knows what's been registered.**

* **Step D: MAPPING_INTENT recording.**  Same for SEM_POOL,
  EXTERNAL_RANGE.  Return `NV_OK`.  This **will break** sem-pool
  mmap until step E lands.

* **Step E: MMAP_REALIZE.**  In `nvkvm_mmap_request` for UVM fds,
  look up the matching intent, send `REALIZE_UVM_MAPPING` to QEMU.
  QEMU does the full kernel sequence in one batched stub call.
  Stub keeps the realized fd alive, returns the realize token.
  Guest module records the token + (gva, gpa, length) in
  `realizations`.

* **Step F: SIDE_EFFECT routing.**  POPULATE_PAGEABLE, MIGRATE,
  VALIDATE_VA_RANGE: forward as `SIDE_EFFECT_UVM` with the
  realize_token of the containing range.  Stub runs the raw ioctl
  on the realize-owned fd.

* **Step G: TEARDOWN.**  UVM_FREE, UVM_DEINITIALIZE, UVM_UNREGISTER_*,
  fd close: send `REALIZE_TEARDOWN` for each live realization;
  clear state.

* **Step H: remove the `uvm_local_fds[]` pool from the stub.**  No
  longer needed once every UVM fd is per-realize.

* **Step I: cataloge & migrate frontend mmap (nvidia0 BAR, nvidiactl
  info page).**  Same shape: MAPPING_INTENT (implicit, single-shot
  on the open) → MMAP_REALIZE.

* **Step J: cataloge & migrate the rest of RM_ALLOC_MEMORY /
  RM_MAP_MEMORY (memory-class RM ioctls that today bypass the
  isolate's pClient validation).**  Move to realize shape; QEMU
  enforces size/offset caps.

* **Step K (long-term):** every guest→host ioctl is either
  `ISOLATE_PASSTHROUGH` (validated by RM's pClient identity check)
  or `REALIZE_*` (validated by QEMU policy).  Document the contract
  in `ARCHITECTURE.md`.

After Step E, `cuCtxCreate` should return 0 (or whatever the next
downstream blocker is — *not* `EBADFD`).  Steps F-G are needed for
correctness once we move past cuCtxCreate (cuMemAlloc, cuLaunchKernel
etc. trigger MIGRATE/POPULATE_PAGEABLE).

## 8. Test strategy

* **Unit tests** in `tests/unit/`:
  * `test_uvm_state`: assert that PURE_CONFIG ioctls record state
    correctly and don't generate virtio traffic.
  * `test_realize_intent_match`: given a recorded intent and an
    mmap request, assert the right intent is matched.

* **Integration tests** in `tests/integration/`:
  * `test_realize_uvm_sem_pool`: end-to-end SEM_POOL realize from
    guest mmap, asserts GPA installed and `mmap()` returns the gva.

* **Existing tests must keep passing**:
  * `test_ioctl_fwd` (current 48/48) — RM passthrough must keep
    working through migration.
  * `host_dump` / `cumemalloc` on guest must reach cuCtxCreate=0
    after Step E lands.

* **Regression**: after each step, run the kernel-side printk
  diagnostic (`docs/kernel_patches/`) and confirm we haven't
  regressed earlier stages.

## 8a. QEMU validation hardening

**Threat model**: assume a fully malicious guest VM.  Any byte of any
field of any struct delivered to QEMU is attacker-controlled.  A
single un-validated value reaching a syscall QEMU executes equals
an attacker getting QEMU's host privileges (which by definition can
access the GPU directly and may be running as root on the host).
The validator's job is to make sure that does not happen — ever, for
any cmd, on any code path, including future ones.  Whitelist
everything; deny by default.

**Isolate-bound ioctls have a softer requirement.**  When QEMU's
role is purely to forward to the stub (no QEMU-side syscall), the
stub IS the validation boundary — sandboxed, no host privileges,
RM-handle-validated at the kernel level.  Strict per-field
validation is still good practice for these (we already restore
embedded-fd round-trips, sanitize VA pointers, etc.) but the
**failure-mode equivalence is different**: a missed validation on
an isolate-bound cmd costs at most a sandboxed-stub compromise;
a missed validation on a QEMU-executed cmd costs host privilege.
Plan accordingly when prioritizing audit effort.

For every ioctl QEMU executes on its privileged side, the
per-cmd validator is exhaustive and self-contained.  No "best-effort"
or "trust the guest" — every byte that reaches the kernel is one
QEMU put there, computed from a guest input that the validator
explicitly approved.

Per-cmd validator MUST:

* **Size**: assert `guest_param_size == sizeof(known_struct)` exactly.
  Reject under-length.  Truncate over-length silently — QEMU's own
  kernel-bound buffer is allocated to the documented size only.

* **Every field**: enumerated in the validator.  No field copied by
  default; every field is either:
    * **passthrough-with-bounds** (e.g. `length <= max_session_quota`)
    * **translated** (e.g. `gva -> host_va` via per-session GVA map)
    * **handle-translated** (e.g. `rm_ctrl_fd_handle -> stub_local_fd`)
    * **flag-filtered** (see below)
    * **must-be-zero** (rejected if non-zero)

  Any field the validator hasn't enumerated is **zeroed** before the
  kernel call.  Adding a new field to a struct in a future kernel
  doesn't accidentally expose anything.

* **Every pointer**: gVisor nvproxy has the reference behaviour
  catalog — for each ioctl, every embedded NvP64 / userspace VA
  pointer is documented as `inout`, `in-only`, `out-only`, list/
  array, optional, etc.  Mirror that table.  In our model:
    * No raw guest VA is ever sent to the host kernel.  Pointers are
      either:
        * replaced with `0` if the target data has been copied into a
          side-channel (aux buffer) the kernel will read directly,
        * or replaced with a host VA that QEMU computed by looking up
          the guest VA in its session-local mapping table.
    * After the ioctl, the response struct's pointer fields are
      restored to the caller's original guest VA before returning,
      per the writeback bug-class fix in [[writeback-bug-pattern]].

* **Every flag**: per-cmd allowlist of bit values.
    * Unknown bits → **rejected** (cmd returns EINVAL).
    * Or, for known-safe-to-ignore bits, **stripped** (silently
      cleared before the kernel call).
    * Default policy is reject; strip is opt-in per bit.

* **Every array / variable-length field**: count is bounded by a
  per-session quota (e.g. perGpuAttributes[]: at most
  `session.max_registered_gpus` entries).  Each array element
  recursively goes through the same field-by-field validation.

* **No transitive trust**: a field validated as a "handle" doesn't
  imply anything about the resource it refers to.  The validator
  re-resolves the handle against the session's tracking tables
  EVERY time, even if the same handle appeared in the same struct
  a millisecond ago.

Concrete artifact: `src/qemu/realize_validators/<cmd>.c` — one file
per cmd, generated from a small schema-like description (similar
to how `nvproxy/seccomp_filters.go` is generated in gVisor).  Each
validator file has a single entry point of shape:

```c
int validate_and_translate_<CMD>(
    struct nvkvm_session *s,
    const void *guest_buf,         /* sized strictly */
    size_t guest_buf_size,
    void *kernel_buf,              /* sized to sizeof(known struct) */
    void *response_remap_table);   /* for writeback restore */
```

Returns `0` on success, `-errno` on rejection.  Test files in
`tests/unit/realize_validators/` exercise every reject branch and
every translation case per cmd.

## 9. Security notes

* QEMU's policy module (new file `src/qemu/realize_policy.c`)
  becomes the single point that validates *all* guest realize
  requests.  Hard caps on per-session totals: max registered GPUs,
  max range groups, max total sem-pool bytes, max live
  realizations, …
* Per-session policy can later be made configurable (mass-market
  configs vs. internal-tool configs).
* Realize tokens are opaque (random 64-bit) — guest cannot forge.
* The stub never sees a guest VA directly; only the QEMU-chosen GPA
  + the host VA QEMU asked it to mmap into.

## 10. Open questions

1. Do any UVM cmds carry RM handle references that need translation
   (the way our sanitizer translates fd→handle_id today)?
   Yes — `UVM_REGISTER_GPU_VASPACE.rm_ctrl_fd` does.  The
   STATE_REGISTRATION path needs to translate at recording time
   and store the handle_id.  At realize time, QEMU re-translates
   to the stub's local fd.
2. Can libcuda mmap a UVM fd at an offset that doesn't match any
   recorded intent (e.g. EXTERNAL_RANGE was created at gva=A, but
   mmap is requested at gva=B)?  Need to check gVisor's nvproxy
   behavior here.  If yes, treat as malformed → return EBADF.
3. What happens if the guest closes the fd while a realization is
   live?  Teardown path must reap all realizations for that fd.
4. `UVM_INITIALIZE` flags differ across libcuda versions — do we
   need a version table, or can we treat them as opaque?  Treat
   as opaque; QEMU just replays whatever was recorded.
5. Does any side-effect ioctl mutate state we track (e.g. MIGRATE
   evicting a range)?  Yes — MIGRATE moves residency.  We track
   intent, not residency; residency lives in the kernel.  No state
   update needed in the guest module.

## 11. Concrete first commits

Once this plan is checked in, the work is unblocked.  In order:

1. `state-machine: scaffold nvkvm_uvm_fd_state` (Step A)
2. `state-machine: PURE_CONFIG short-circuit for UVM` (Step B)
3. `state-machine: record STATE_REGISTRATION` (Step C)
4. `state-machine: record MAPPING_INTENT for SEM_POOL` (Step D)
5. `state-machine: REALIZE_UVM_MAPPING ABI + handler` (Step E core)
6. `state-machine: stub REALIZE_UVM_FD execution path` (Step E stub)
7. `state-machine: end-to-end mmap-realize for SEM_POOL` (Step E
   integration — cuCtxCreate must return 0)

Each commit small, each compileable, each shippable.

---

**See also**: `[[gpfifo-schedule-runlist-bug]]`, `[[dma-copy-class-alloc-params]]`,
`[[writeback-bug-pattern]]`.

## 12. Session ending state (2026-05-28)

Landed and verified on `nvkvm-tables-refactor`:

- **Step A** (6dcc36f) — `nvkvm_uvm_fd_state` scaffolding.  Allocated
  in `nvkvm_open` for UVM fds, freed in `nvkvm_release`.
- **Step B** (aa32a26) — `UVM_INITIALIZE` flags recorded into
  `uvm_state->init_flags`.  Forwarding kept (additive).
- **Step C** (6a665b4) — `UVM_REGISTER_GPU` / `_GPU_VASPACE` /
  `_CREATE_RANGE_GROUP` recorded into per-fd lists.  Forwarding kept.
- **Step D** (b25de6a) — `UVM_ALLOC_SEMAPHORE_POOL` recorded as a
  `nvkvm_uvm_mapping_intent { mode=SEM_POOL, base, length, params }`.
  Forwarding kept.
- **Step E (1/4)** (b1228e3) — `NVKVM_REQ_REALIZE_UVM_MAPPING` wire
  protocol in `src/common/nvkvm_proto.h`:
  request, response, state snapshot, mode constants, caps.

In-flight (3 commits left for Step E):

- **Step E (2/4)** — stub-side `ISOLATE_CMD_REALIZE_UVM_FD` handler:
  open fresh UVM fd, replay `UVM_INITIALIZE`+`UVM_REGISTER_GPU(*)`+
  `UVM_REGISTER_GPU_VASPACE(*)`+`UVM_CREATE_RANGE_GROUP(*)`+
  the mode-specific intent, then `mmap(2)` and return host VA +
  realize_token.
- **Step E (3/4)** — QEMU-side handler: validate strictly per §8a
  (size, fields, flags, pointer translation, allowlist), allocate
  GPA from the session GPA window, install KVM region after stub
  returns host VA.
- **Step E (4/4)** — guest module dispatch: in
  `nvkvm_mmap_request_isolate` for UVM fds, look up matching intent
  in `uvm_state->intents`, call new `nvkvm_virtio_realize_uvm_mapping`
  instead of the standard `mmap_on_isolate`.  `remap_pfn_range` the
  returned GPA.  Record into `uvm_state->realizations` for teardown.

Note on the immediate-unblock alternative: a much smaller change in
the stub (pre-`UVM_INITIALIZE` each `uvm_local_fds[]` pool slot at
boot) would unblock the `EBADFD` mmap right now, without the full
state-machine refactor.  I attempted this but the stub uses
`-nostdlib` and its `syscall()` linkage needs the larger
QEMU-build pipeline to verify.  Decision: skip the transitional
fix; land Step E proper in the next session.  The protocol contract
is committed (b1228e3), the recording layer is committed, only the
realize execution path remains.
