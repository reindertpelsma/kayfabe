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

## 1. The four axes

| axis | varies with | where it lives | status |
|---|---|---|---|
| **GPU architecture** | the silicon (GA106, Ada, Hopper…) | `kayfabe-arch`: `Arch`, `GmmuFmt`, `UserdModel`, `GspModel` | seam exists, **one** impl |
| **Guest driver version** | the driver *in the guest* | `kayfabe-abi::DriverVersion` — a full `(major, minor, patch)` triple with a loud `NoTableForVersion` floor | seam exists, few tables |
| **Host driver version** | the driver *on the host* | implicit: `VerbPlan` carries **named intents** that *"the adapter lowers to the correct per-version NVOS sequence"* (`kayfabe-isolate`) | abstract by construction, **unbuilt** |
| ★ **Guest OS** | Linux / Windows / … | **nowhere yet** — `DriverVersion` silently means "Linux, ogkm-shaped" | **not a seam yet** |

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

**Known layout breaks** (each costs one table entry, mechanical):
- `NVOS46` 56 → 64 bytes at **580.65.06** (`docs/reference/nvidia_abi_oracles.md` F1)
- the GSP queue element 48 → 16 bytes with MCTP/NVDM headers somewhere in **(570, 610]**

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

## 5. What this forbids

1. No `if guest_version == …` in a logic crate. Transitions fire on **observed protocol facts**
   (what the guest wrote/posted/declared), never on driver identity — the protocol-not-trace
   doctrine, applied to bring-up where an identity check is most tempting.
2. No chip constant in a logic crate — it goes behind `Arch` (`kayfabe-gsp` is a logic crate).
3. No OS assumption without a comment naming it, so the future Windows seam is a grep away.
4. Any bootstrap we do not support is a **refusal at realize**, not a partial attempt.
