# The host driver version — a disjoint axis, and the pin that was silent

> **Status.** The property is intended and is a product advantage. The *implementation* is
> **half-built**, and this note exists so the aspiration is never read as the state.
> Landed 2026-07-31: the pin is now a **named refusal** rather than a silent constant. The
> host version **axis** is still unbuilt, on purpose (§4).

## 0. The property

**The guest driver version and the host driver version are disjoint.**

Mode 2 does not replay the guest's ioctls on the host. The guest runs a stock NVIDIA driver
against our *emulated* GPU; we recover its **intent** from the RM/GSP protocol and re-issue
that intent against the host driver through unprivileged userspace ops. There are therefore
two independent translations with an abstract middle, and the version at each edge is a
separate fact:

- what the **guest** runs is the guest's business — whatever the customer installed;
- what the **host** runs is ours — whatever we ship against.

That is a capability Mode 1 structurally cannot have (it forwards the guest's own ioctls, so
the two ABIs must agree), and it is why `four_axes_of_variation.md` lists them as two axes
rather than one. §4 of that note adds the neighbouring property: we do not depend on the
host's **libcuda** either, because we forward below CUDA.

⚠ **Read §1 before quoting any of the above as a capability.** Only one of the two axes
exists in code.

## 1. What is actually true today

Three readings of this tree, taken on 2026-07-31. They are **source reads, not
measurements** — no host driver other than 580.159.04 has ever been run against this code,
so nothing below says what a mismatched host *does*, only what we would send it.

### 1.1 The guest axis is built

`kayfabe-abi/src/lib.rs` — `DriverVersion` is a full `(major, minor, patch)` triple, and its
rustdoc says the intended thing in as many words:

> *"A guest driver version, as detected/advertised at device realize. (Values are data, not
> code: one generated module per version.)"*

`kayfabe-abi/src/versions.rs` carries the tables it selects between, including a real
in-major transition (`NVOS46_PARAMETERS` 56 → 64 bytes at 580.65.06, kept behind
`MapDmaWire`), and a version with no table is a loud `NoTableForVersion` floor rather than a
guess.

### 1.2 The host axis does not exist

`kayfabe-isolate-host/src/rm.rs` — the crate that actually drives the GPU — encodes **every**
host-side RM parameter block with a const-size, version-free encoder: a `…::SIZE` buffer and
an `encode_into`, used unconditionally. There is no host-side version type, no table, no
selector, and no parameter on any encoder. Before this note's change, the one place a host
version appeared at all was rung R2, which read the string and threw it away
(`read_version(&ctl).unwrap_or_default()`).

### 1.3 The pin is concrete, and it is narrower than "580"

`kayfabe-abi/src/submit.rs` documents its own provenance — *"everything here is read off the
bench's own driver, `ogkm-580: 580.159.04`"* — while the axis table in
`four_axes_of_variation.md` described the host axis as *"abstract by construction,
**unbuilt**"*. Both sentences are true of different things, and together they read as "not
yet needed" when the actual state is "pinned, and nothing says so".

Two transcribed deltas set the interval:

| delta | where | consequence of getting it wrong |
|---|---|---|
| `NV_CHANNEL_ALLOC_PARAMS` gains `hHandleVASpace` at **+32** from 610 (`ogkm-610: src/common/sdk/nvidia/inc/alloc/alloc_channel.h:296-347` vs `ogkm-580: .../alloc_channel.h:296-342`) | every field from +32 onward moves 4 bytes; `engineType` +128 → +132 | the channel is created on a **different runlist** — the C's `engineType = 0` bug class (`dma_copy_class_alloc_params`, seam audit GR-1) reached from another road |
| `NVOS46_PARAMETERS` 56 → 64 bytes at **580.65.06** (`kayfabe-abi/src/versions.rs`; `docs/reference/nvidia_abi_oracles.md` F1) | the guest side keeps both forms behind `MapDmaWire`; the host side writes the 64-byte form unconditionally | a host below 580.65.06 is handed a map-DMA block eight bytes longer than its frontend sizes |

⇒ the host-side encoders are pinned to the interval **`[580.65.06, 581.0.00)`**, not to a
major and not to "recent".

### 1.4 ⇒ The failure mode was SILENT

A host driver outside that interval did not produce a refusal. It produced **wrong offsets
on successful ioctls** — the plausible failure, which is the expensive one. The guest axis
refuses loudly when it has no table; the host axis had nothing to refuse with.

## 2. What landed — the pin says its own name

`kayfabe-abi/src/host_driver.rs` (data crate: a version fact is data) holds the interval, a
`HostDriverVersion` type distinct from the guest's `DriverVersion`, and `check()`.
`kayfabe-isolate-host`'s rung **R2 is now a gate**: it asks the host frontend its version and
refuses by name if it is one these encoders do not cover. The refusal names the host, what
we emit, and the concrete delta:

```
RM bring-up failed at R2 host driver version: host driver is 610.43.02;
kayfabe-isolate-host's encoders emit 580.65.06-era struct layouts (the interval
[580.65.06, 581.0.00)) (NV_CHANNEL_ALLOC_PARAMS gains hHandleVASpace at +32 from 610,
shifting engineType from +128 to +132 — a channel encoded at 580 offsets and read by a
610 driver names a different runlist); refusing rather than encoding wrong offsets.
```

This is the house style — `RefusingRam`, `GspFault::GuestRam`, the GSS-legacy refusal at
`8938491` — and the argument for it is epistemic rather than aesthetic: **a refusal is not
plausible, so it surfaces at the rung that made it; a wrong offset is plausible, so it
corrupts quietly several layers away.**

### 2.1 Where the version comes from, and why not the other two sources

`NV_ESC_CHECK_VERSION_STR` on `/dev/nvidiactl`, **query form** (`cmd = '2'`).
`RmPerformVersionCheck` answers that form by copying `NV_VERSION_STRING` into the reply and
returning `NV_OK` (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osapi.c:1066-1071`), so the
answer is the RM's own version string.

| candidate | verdict |
|---|---|
| `/proc/driver/nvidia/version` | ⊘ **unreachable at the point of use.** The isolate child runs inside a `pivot_root`ed sandbox whose scratch root is mounted **over `/proc`** and whose old root is then detached (`kayfabe-linux-raw/src/sandbox_unsafe.rs`, `SCRATCH = "/proc"`). The process that issues the encoded ioctls has no `/proc` at all. |
| module version (`modinfo`, sysfs) | ⊘ describes the module on disk, not the frontend this session is bound to — and needs a filesystem the sandbox does not have either. |
| ★ `NV_ESC_CHECK_VERSION_STR` | **chosen.** It is answered on the *same descriptor* the encoders are about to be used on, it works inside the sandbox, and it names the RM that will parse our structs rather than a module that happens to be installed. |

### 2.2 ⚠ Unreadable is a refusal, not a default

Three distinct refusals, and the distinction is the point:

| input | refusal | why its own arm |
|---|---|---|
| the ioctl failed | `Unreadable` | this is the arm that must never collapse into "assume 580". The code it replaces produced `""` here and continued. |
| a reply that is not `major.minor.patch` | `Unparsable` (carries the string) | an unrecognised answer is *less* evidence than no answer, not more — and the two need different fixes |
| a readable version outside the interval | `NotEncodedFor` | the only arm where we know both numbers, so it is the only one that can name the delta |

**No override exists.** No environment variable, no flag, no force argument. A refusal that
can be switched off is one that will be switched off by whoever meets it at 2am, and what is
behind this one is silent corruption on the host's own GPU. The way past it is §5.

## 3. The bite — this refusal has been SEEN to fire

⊘ A refusal path that has never fired is not evidence. Both directions were run on real
hardware on **2026-07-31**: an RTX 3090 with host driver **580.159.04**, base commit
`d89c73a`, driving `crates/kayfabe-isolate-host/src/bin/rmladder.rs` — the project's own
bring-up ladder, not a harness written for the occasion.

**3.1 Accept.** The ladder walked the whole way against the real driver and reported the
host's version at R2. First and last lines of the run (RTX 3090, 580.159.04):

```
ok    R2 version         = "580.159.04"
…
★     R17 CE COPY         = 4096 bytes: dst[0] 0x3f0011ff -> 0xc0ffee00, dst[last] 0xc0fff1ff (want 0xc0fff1ff) — read back through an INDEPENDENT mapping
done                                                                  [rc=0]
```

**3.2 Refuse — a faked host version, injected at the ioctl boundary.** `read_version` was
temporarily made to return `Some("610.43.02")` — the *other vendored tree's* version, on the
**same** RTX 3090 — and the ladder rebuilt and re-run on 2026-07-31. It stopped at R2, before
a single encoded parameter block reached the driver:

```
FAIL  RM bring-up failed at R2 host driver version: host driver is 610.43.02;
kayfabe-isolate-host's encoders emit 580.65.06-era struct layouts (the interval
[580.65.06, 581.0.00)) (NV_CHANNEL_ALLOC_PARAMS gains hHandleVASpace at +32 from 610,
shifting engineType from +128 to +132 — a channel encoded at 580 offsets and read by a
610 driver names a different runlist); refusing rather than encoding wrong offsets.
                                                                      [rc=1]
```

**3.3 Refuse — the frontend answers nothing.** The same injection point was then made to
return `None`, which is exactly what the replaced `.unwrap_or_default()` used to swallow.
Same RTX 3090, same day:

```
FAIL  RM bring-up failed at R2 host driver version: the host driver did not answer
NV_ESC_CHECK_VERSION_STR, so there is no host driver version to check;
kayfabe-isolate-host's encoders emit 580.65.06-era struct layouts (the interval
[580.65.06, 581.0.00)), and an unread version is not evidence of one; refusing rather
than assuming 580.
                                                                      [rc=1]
```

**3.4 Induction removed, accept re-run.** The injection was reverted, the file checksummed
against the committed one, and the ladder re-run on the same RTX 3090 / 580.159.04 box: back
to `ok    R2 version         = "580.159.04"` and `rc=0`.

**3.5 What the standing tests hold.** The ioctl itself is the only part of R2 they cannot
reach: `kayfabe-abi/src/host_driver.rs`'s six tests drive `check()` directly, and
`kayfabe-isolate-host/src/rm.rs`'s `r2_admits_the_benchs_host_driver_and_refuses_silence`
and `a_refused_host_driver_arrives_as_a_named_r2_failure` drive the gate function and the
`BringUpError` a human actually sees.

★ None of them reads a golden back at the constant under test: every version in them is a
literal tag (`580.159.04`, `580.65.06`, `580.65.05`, `610.43.02`, `575.64.05`, `590.44.01`),
and the assertions are over the refusal's **text**, not over `ENCODED_FOR_MAJOR`.

## 4. ⊘ Why a host version axis was NOT built

Considered and rejected, and the reasoning is a constraint on anyone who reads this note as
a to-do:

**We have exactly one host driver.** A version axis we cannot exercise is a mechanism with
**no red available to it** — precisely what the arch-axis experiment found with Ada (run
named: task #118, commit `554c333`, pinned by `tests/tests/arch_axis_second_generation.rs`;
written up at `open_questions_for_the_owner.md` §"What was run"), whose verdict was *"an
experiment that selects its easiest case produces a green with no red available to it"*.
Every table but one would be unexercised; every test over it would pass for the wrong
reason; and the abstraction would land in `kayfabe-isolate-host`, the piece that actually
drives the GPU, two rungs from first compute.

The refusal costs one comparison and buys the thing the axis was wanted for: a mismatched
host is **detected**, by name, at the first rung that touches the driver.

## 5. What remains pinned — the follow-on, named not built

1. ★ **`ChannelAllocParams` takes no version parameter.** It is an offset-addressed encoder
   over 580's field placement, used unconditionally by `kayfabe-isolate-host`. Folding a
   version parameter into it has blast radius outside `kayfabe-abi` and is deliberately
   **not** done here (it is also the deferred item recorded at
   `open_questions_for_the_owner.md` §Q11). The same is true of `Nvos46Parameters`,
   `Nv0080AllocParameters` and every other `…::SIZE` buffer in `rm.rs`.
2. **Nothing selects on the host version.** `RmConnection` stores the string; the value of
   the check is entirely in the refusal.
3. **The supported *drift range* is still unmeasured.** `four_axes_of_variation.md` §3's
   estimate stands, and the experiment it proposes — pin the host, walk the guest back — is
   unaffected by this note: this gate is about the **host** edge only.
4. When a second host driver becomes available, the honest next step is one more interval in
   `host_driver.rs` plus the encoders that interval needs — at which point the axis has a
   red available to it and can be built against a real disagreement.
