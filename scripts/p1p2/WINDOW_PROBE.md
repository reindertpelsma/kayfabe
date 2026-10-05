# Fault-aware USER-channel window probe

**STATUS: RESEARCH, 2026-10-05.** Implemented and GPU-free tested, **not run on
hardware**. This is preparation for T-WINDOW-USER, not a completed isolation
gate. It does not implement T-PHYS-CE or T-RING-TRANSLATED. No default changes.

`kf-window-probe` uses the existing v3 `kf-host` verbs, `kf-harness` CE packet
encoder, generated engine/class selection and `kf-abi` notifier layout. It
does not change or depend on the frozen `kayfabe-*` tree. The new product helper
is a bounded decoder beside the existing 16-byte error-notifier encoder.

## What it actually proves

The probe creates two independent VA spaces, TSGs and USER channels. Each has
one private 64 KiB allocation for its pushbuffer, GPFIFO, USERD, error record,
source, destination and fences. Channel births go through the existing P0
CAP_SYS_ADMIN bracket and USER-reply check. The executable also refuses a
root UID or nonzero effective capability set before opening a GPU.

Both channels first perform a known-good virtual CE copy. The victim then
reads **exactly four bytes** from the test window into its own poisoned
destination. All destinations, semaphore releases and host-WFI fence releases
are inside its own allocation. It never submits a physical-mode operand.

An expected readable window passes only with both real GPU completion values,
the exact random canary word, no channel error, and a successful neighbor copy
afterward. An expected denial passes only when all of these hold:

- The victim's own native context-DMA notifier publishes status `0xffff`, the
  source-generated `ROBUST_CHANNEL_FIFO_ERROR_MMU_ERR_FLT`, and COPY0's engine
  identity. Another RC reason is a failure, not equivalent denial evidence.
- Neither CE completion nor host-WFI fence arrives; the destination remains
  poisoned and the source canary is unchanged.
- The independent neighbor still copies and releases its fences. A GPU-wide
  failure or timeout cannot satisfy this check.
- The victim is sampled again after the neighbor's fence. A late leaked word
  or completion invalidates the initial observation.
- Channel, view, mapping, allocation and VA-space cleanup calls succeed.

GPU waits are bounded to three seconds per submission; the manifest wait is
60 seconds. A timeout alone always fails. Capture an outer process timeout as
a failed/inconclusive run too: RM ioctls and teardown are kernel operations and
are not claimed to have a userspace-enforced timeout.

## Native instrumentation control comes first

Build only this harness binary, recording the exact git revision:

```sh
git rev-parse HEAD
CARGO_BUILD_JOBS=2 cargo build -p kf-harness --bin kf-window-probe
```

On an exclusively assigned test GPU, as an ordinary user with GPU-device
permissions and no capabilities:

```sh
timeout --signal=TERM --kill-after=10s 90s ./kf-window-probe --self-test --gpu-minor 0
```

This mode creates a second alias of **its own allocation**, verifies a canary
read, unmaps that alias, and requires the following access to produce the
specific MMU fault while the neighbor remains live. RM chooses the alias;
no address or register row is copied from another die. The alias is allocated
bottom-up only to keep this instrumentation control near the ordinary ring;
it does not imitate production window placement.

**A self-test pass validates the notifier/neighbor instrument. It does not
exercise QEMU's decision to omit windows and cannot close S1-21.** A failure
stops the later guest experiment. Preserve the source revision, driver, GPU,
UID/capability line, birth replies, full probe output and host Xids. Hardware
permission, notifier publication, fault engine identity and teardown remain
unverified until this native control runs on each tested family/driver.

## Guest window fixture protocol

Run a fresh Linux guest with the exact candidate binary, `KF3_MAPLOG=1`, and
the chosen `KF3_TSPACE` setting. Run the probe as an unprivileged guest user:

```sh
timeout --signal=TERM --kill-after=10s 90s ./kf-window-probe \
  --manifest /tmp/this-run-window.fixture --gpu-minor 0
```

The path must not already exist. After baseline copies, `WINDOW_READY` prints
an unpredictable run nonce, the guest VA-space/channel/memory handles, the
source VA, offset and random canary. The trusted host coordinator must then:

1. Associate that exact live guest space with its host mirror in this VM. Use
   the full `VasKey` and contemporaneous allocation/map logs; a low handle by
   itself is not sufficient across multiple clients. Resolve `source_va` from
   an applied, current map run, including its aperture and offset.
2. For the positive arm, use that mirror's actual RM-chosen framebuffer
   window base/length. Derive the canary's offset from the current source row.
   The canary must be the probe-owned word announced at READY. Do not insert
   a previously captured physical address or a convenient fixed window base.
3. For the negative arm, pair the old-window test address with recorded
   positive evidence and the current canary placement. Independently verify
   `windows=none` for every current mirror/spare and absence of an ordinary
   guest PTE at the tested address. A relocated window elsewhere would escape
   a spot check; the source review and full `tspace_log_gate.py --user` remain
   required. Run the dedicated `KF3_NEGCTL_TWIN_WINDOW=1` control as well.
4. Publish a regular file atomically under the specified guest path. The
   coordinator owns the file and must not replace its type or alter it after
   publication. The parser refuses unknown/duplicate keys, stale run/space
   identity, input above 4 KiB and unaligned/out-of-range/overflowing extents.

The exact seven-line format is:

```text
format=kf-window-v1
nonce=<decimal or 0x value from WINDOW_READY>
space=<decimal or 0x guest space from WINDOW_READY>
base=<RM-chosen window base from this VM's evidence>
len=<window length from that evidence>
offset=<current probe canary offset within the window>
expect=<read or fault>
```

The angle-bracket expressions above are placeholders, not literal valid
inputs. `base + len` must fit below `2^40`; no test read may overlap the
probe's ordinary mapped allocation. The fixture is a trusted test input,
not a new guest-to-host command channel or a product interface. Supplying
plausible integers does not establish their provenance.

**The host coordinator is still to be implemented and reviewed.** The binary
does not invent a per-space identity, canary offset or missing window base.
The current guest probe covers a framebuffer canary owned by the probe.
Guest-RAM, page-table and carve-out-root targets from the full P1/P2 design
still need separate owned fixtures and provenance checks. Do not mark the
full T-WINDOW-USER table row complete on this preparatory binary alone.

## Physical and Translated arms: unresolved prerequisites

T-PHYS-CE must not use guest GPGA as host physical VRAM. Those are different
address domains; a guessed operand could touch memory outside the test's
allocations if the protection under test fails.

OGKM 580.159.04's `ctrl0041.h` documents
`NV0041_CTRL_CMD_GET_SURFACE_PHYS_ATTR` as MODS-oriented. Its generated
`g_mem_nvoc.c` entry for `0x410103` has flags `0x4` (privileged), and
`memCtrlCmdGetSurfacePhysAttrLvm_IMPL` in
`src/nvidia/src/kernel/gpu/mem_mgr/mem_ctrl.c` obtains a physical address from
the allocation's memory descriptor. This is not an unprivileged production
oracle. No privileged control was added to `HostRm` or its allowlist.

A future physical test needs a trusted host fixture that owns and pins a
canary allocation, derives its actual address and aperture through an audited
native mechanism, keeps it alive throughout the test, and proves the complete
operand stays inside it. It also needs the family-specific CE error criterion
and both privilege arms, followed by the specified reset. The guest receives
only a dedicated test target, never a raw arbitrary-address option. Until
that fixture exists, **there is no physical-mode submission in this probe**.

The Translated channel/ring arm separately requires the planned guest-kernel
module and private T-space canaries. A raw USER-channel fault cannot certify
the Translated re-authoring path. These prerequisites are research work, not
permission questions or a reason to relax v3's host boundary.

## Local verification

The GPU-free tests inject missing/wrong faults, leaked bytes, successful
completion after an alleged fault, dead neighbors, bad baselines and malformed
manifests. Every such false-positive path fails. They also check every
truncation of the notifier decoder. No hardware result is represented by these
unit tests. See the branch's `traces/tspace_probe_preparation_20261005/` for
revision-stamped build/test evidence.
