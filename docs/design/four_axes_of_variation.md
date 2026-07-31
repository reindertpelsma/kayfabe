# The four axes of variation — and why guest/host driver mismatch is a FEATURE

> **Owner directive (2026-07-27):** *"I want guest host driver mismatch [to] be a feature to
> support, especially for small drifts, we should test it as well… ensure it does not become [a]
> bolt on. not for all mismatch, like only ogkm-like bootstrap sequences. this is necessary I
> think to also later support Windows."*

## 0. The claim, in one paragraph

Mode 2 does **not** replay the guest's ioctls on the host. It reconstructs *intent* from the RM
protocol and re-issues that intent against the host driver. So the guest edge and the host edge
are **two independent translations with an abstract middle**, and a guest/host driver mismatch is
a translation problem at two well-defined seams rather than a passthrough problem. That is a
capability Mode 1 structurally cannot have — it forwards the guest's own ioctls, so guest and host
ABIs must agree.

⚠ **Read `host_driver_version_pin.md` before quoting that as a shipped capability.** The
guest half of the disjointness is built; the **host** half is not. The host-side encoders are
pinned to one driver interval, which until 2026-07-31 nothing said and nothing checked. That
note states the property, states what is actually true, and records the refusal that now
makes a mismatched host stop at rung R2 instead of receiving wrong struct offsets.

★ **Six axes now, and two of them are audited elsewhere.** `compatibility_matrix.md`
(2026-07-31) adds **guest kernel version** and **multi-GPU**, states every cell of the
six-axis matrix as built / designed-only / unknown / known-broken, and — the part that
reframes the four below — derives which crates a shipping artifact actually **links**.
Read it before quoting any status here: three of the four axes' seams live in crates no
build output links today, and `scripts/compat_matrix.py` regenerates that answer rather
than restating it.

## 1. The four axes

| axis | varies with | where it lives | status |
|---|---|---|---|
| **GPU architecture** | the silicon (GA106, Ada, Hopper…) | `kayfabe-arch`: `Arch`, `GmmuFmt`, `UserdModel`, `GspModel` | seam exists, **one** impl |
| **Guest driver version** | the driver *in the guest* | `kayfabe-abi::DriverVersion` — a full `(major, minor, patch)` triple with a loud `NoTableForVersion` floor | seam exists, few tables |
| **Host driver version** | the driver *on the host* | ★ **CORRECTED 2026-07-31** — see `host_driver_version_pin.md`. `VerbPlan` carries named intents, but the thing that lowers them (`kayfabe-isolate-host/src/rm.rs`) uses **const-size, version-free** encoders, so the host edge is concretely **pinned to `[580.65.06, 581.0.00)`** | axis still **unbuilt** — and deliberately so — but the pin is now a **named refusal at rung R2** instead of silently wrong offsets |
| ★ **Guest OS** | Linux / Windows / … | `kayfabe-abi::GuestOs` (`guest_os.rs`) — a profile **beside** the version table, declared at realize; every OS-conditional rule is data on it, and an OS with no rule is a typed refusal | ★ seam exists (2026-07-29), **one** profile + one named refusal |

★ **The fourth axis is the one to protect now**, because it is the only one with no home. A
Windows guest runs an entirely different driver, but the **GSP RPC protocol and the RM object
model are largely OS-independent** — the ioctl/escape layer above them is what differs. So Windows
should later be *another guest-side implementation behind the same seam*, not a rewrite of the
boot FSM. That only stays true if nothing bakes in "the guest is Linux".

### 1.1 ★★ Windows **guest** is a target. Windows **host** is explicitly NOT.

> **Owner (2026-07-27):** *"windows as guest is way more important than host (since windows host
> already has hyper-v pv for gpu and target audience is smaller)."*

The two are not symmetric, and conflating them would cost far more than the guest work itself:

| | what it would take | verdict |
|---|---|---|
| **Windows GUEST** | another **guest-side** decode/protocol implementation behind the fourth axis. The GSP protocol and RM object model are shared; the escape/ioctl layer differs. Everything below the seam — core, isolates, host edge — is **unchanged**. | ★ **TARGET.** Keep the seam clean; do not implement yet. |
| **Windows HOST** | the isolate would have to run on Windows and drive the *Windows* RM: no `/dev/nvidiactl`, a different escape mechanism, a different process/handle model, and the whole unprivileged-isolate design re-founded on Windows primitives. That is a **second host port**, not a seam. | **OUT OF SCOPE**, and stays out. Windows hosts already have Hyper-V GPU-PV, and the audience is smaller. |

**The consequence for design work:** when something must be OS-specific, ask *"guest side or host
side?"* — guest-side OS-specificity goes **behind the fourth-axis seam**; host-side OS-specificity
may be written **assuming Linux**, freely and without apology. `kayfabe-linux-raw` is named for
that reason and needs no abstraction for a hypothetical Windows host.

**Do not collapse guest OS into the version key.** Conflating them is the C's major-only version
key mistake one level up: a single key that silently spans two independent dimensions, so a
mismatch on one is mis-served by a table chosen for the other.

## 2. Scope — deliberately bounded

Support drift **only across ogkm-like bootstrap sequences**. Anything requiring a genuinely
different bootstrap — pre-GSP drivers (roughly ≤ 510–515 on consumer), or a different handshake
entirely — is **out of scope and must be a LOUD REFUSAL**, never a best-effort attempt. A
best-effort bring-up against an unsupported bootstrap fails deep inside the guest driver with no
useful diagnostic; a refusal at realize costs one line of log.

## 3. The asymmetry that sets the range

**Old guest on new host is the safe direction.** RM keeps its userspace ABI backward-compatible,
so an older guest asks for things a newer host still provides. **New guest on old host is where it
breaks** — the guest can request classes, controls or capabilities the host does not have, and the
honest answer is a refusal, not emulation.

★ **Both breaks below are host-edge facts as well as guest-edge ones**, and that was not
written down until 2026-07-31. `kayfabe-isolate-host` emits the post-580.65.06 `NVOS46` form
and 580's `NV_CHANNEL_ALLOC_PARAMS` offsets unconditionally, so each break bounds the range
of **hosts** we may point those encoders at — see `host_driver_version_pin.md` §1.3.

**Known layout breaks** (each costs one table entry, mechanical):
- `NVOS46` 56 → 64 bytes at **580.65.06** (`docs/reference/nvidia_abi_oracles.md` F1)
- the GSP queue element 48 → 16 bytes with MCTP/NVDM headers in **(595.84, 610.43.02]**
  — narrowed 2026-07-28 from the earlier estimate of `(570, 610]`. The 48-byte form with
  `elemCount@40` is present at 575.64.05, 580.65.06, **580.159.04**, 580.173.02, 590.44.01,
  590.48.01, 595.44.02 and 595.84; the 16-byte MCTP/NVDM form appears only at **610.43.02**.
  ⇒ **580, 590 and 595 are all on the 48-byte side**, and the version predicate is
  `major >= 610`, not `> 570`. Only 580.159.04 and 610.43.02 are vendored here
  (`research_clones/ogkm-580.159.04/`, `research_clones/ogkm/`) and were read directly; the
  other seven tags are relayed. See `mode2_gsp_port_plan.md` §4.3 and §14.4.

★ **This break is bigger than a layout entry, which is why it is worth naming here.** It is
not only field offsets: at 610 the receiver *derives* the element count from `rpc.length`,
while below 610 it *reads* an `elemCount` field — and that number is what advances the ring.
So the same "one table entry" carries a **behavioural** difference on both the send and the
receive side, plus a guest-memory-safety bound (`mode2_gsp_port_plan.md` §4.6). A layout table
that only carries offsets is not sufficient for this axis.

★ **What is now measurable rather than estimated.** Two of the three items above are read from
driver source at a named tag, so they are facts about the *guest driver*, not about our stack:
the `NVOS46` boundary and the GSP element break. **That is all.** They bound where a *layout*
changes; they say nothing about whether a mismatched pair actually runs, because nothing in
the Rust stack has touched a GPU. The supported-drift range below therefore stays UNMEASURED —
knowing where the tables must differ is not the same as knowing that a guest at one version
boots against a host at another.

**Estimated range, explicitly UNMEASURED:** comfortable within a major (580.x guest / 580.y host),
plausible one major back (575 guest / 580 host), unlikely forward. Nothing in the Rust stack has
touched a real GPU, and it is not known whether the C ever ran guest and host *mismatched*.

**The experiment**, once GSP boots: pin the host at 580 and walk the guest driver back through
575/570 until something refuses — and record **what** refuses. That yields a real supported range
instead of a guess, and the refusals are the interesting data.

## 4. ★ A property worth stating: we do not depend on the host's libcuda

We forward at the **RM ioctl / GSP** level, below CUDA. The guest brings its own libcuda **inside
the guest**; the host **never runs libcuda at all** — the isolate issues raw RM ioctls. So the
host's CUDA userspace version is simply not a variable.

That is a real advantage over API-proxying approaches (which must track CUDA's surface release by
release), and it is why the axis table above has no CUDA row. It also means a guest can run a
CUDA version the host has never had installed.

## 4.5 ★★★ What the fourth axis actually cost, measured on 2026-07-29

The seam audit costed this axis at *"~100 lines across 3 files, zero trait changes below the ABI
seam"*. That was right about the size and **wrong about the blast radius**, in a direction worth
recording.

**The one violation.** `client_kind_from_process_id` — the wire→`ClientKind` translation, i.e.
the function that decides which RM clients share a host isolate — applied a rule the guest driver
gates on `RMCFG_FEATURE_PLATFORM_UNIX` (`ogkm-580: src/nvidia/inc/kernel/vgpu/rpc.h:67-77` /
`ogkm-610: rpc.h:67-77`, byte-identical) to **every** guest, silently. On a WDDM guest the `else`
arm runs for kernel-privileged clients too, so they declare a real pid.

**The blast radius is not one process.** `ClientKind::User` is not the isolate key — it is the
*eligibility predicate* for a `DUP_OBJECT`-driven merge. Every guest CUDA process dups into the
one kernel/UVM session client (`[measured]`: two concurrent processes, 82 dups each, every one
into that client). On a UNIX guest that client is `ClientKind::Kernel`, is not merge-eligible, and
the dups merge nothing — which is exactly what fixes #14. On a WDDM guest it would have been
merge-eligible, and **every process in the guest plus the guest kernel would have landed in one
host isolate**: #14 un-fixed, silently, on a guest nobody had booted yet. Pinned by
`a_kernel_client_that_declares_a_real_pid_collapses_the_whole_guest_and_only_the_profile_stops_it`.

**★ A SECOND gate on the same field, and it is not the OS.** The `RMCFG_FEATURE_PLATFORM_UNIX`
test sits inside `if (!IsT234DorBetter(pGpu))` (`ogkm-580: rpc.h:57` / `ogkm-610: rpc.h:57`), and
the params struct is zero-initialised at `rpc.h:53`. So on Orin-class silicon RM never writes
`processID` at all and **every** client declares `0` — which today's rule reads as
`User { pid: 0 }` for all of them, collapsing the whole guest by a *chip*-axis condition rather
than an OS one. Recorded, not fixed: our target is GA106, and inventing a rule for hardware we
have never observed is the mistake the Windows arm exists to refuse. The characterisation test is
`a_zero_process_id_is_a_user_client_today_and_that_is_wrong_on_t234d`.

**The lesson for the remaining axes.** One declared wire field was conditioned on two independent
axes, and the code that read it named neither. The cost of the seam really was ~100 lines; the
cost of *finding* it was an audit. Rule 3 below is now gated so the next one is a test failure.

## 5. What this forbids

1. No `if guest_version == …` in a logic crate. Transitions fire on **observed protocol facts**
   (what the guest wrote/posted/declared), never on driver identity — the protocol-not-trace
   doctrine, applied to bring-up where an identity check is most tempting.
2. No chip constant in a logic crate — it goes behind `Arch` (`kayfabe-gsp` is a logic crate).
3. No OS assumption without a comment naming it, so the future Windows seam is a grep away.
   **Gated since 2026-07-29** by `tests/tests/guest_os_axis_gate.rs` — a Rust test rather than a
   `ci.yml` step, because it runs its own checker against a synthetic violation per token and so
   can prove it is able to fail, which a YAML `grep` cannot.
4. Any bootstrap we do not support is a **refusal at realize**, not a partial attempt.
