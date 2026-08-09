# VMM integration and the support matrix — a DECISION RECORD

> **Status: DECIDED by the owner, 2026-08-09.** This file is a decision record, not a proposal.
> Nothing below is open. If a later document, code comment or agent brief takes a contrary
> position on **how we integrate with a VMM** or on **which axes may carry a version floor**,
> that document is wrong and this one governs — say so and fix it there, rather than
> re-opening the question here.
>
> ⚠ **Three prior sites contradicted each other on exactly this** before this record existed
> (see §6). A contradiction that survives in four places is not a disagreement, it is an
> absence of a decision. This is the decision.

---

## 0. The three rulings, in the owner's words

### Ruling 1 — VMM integration must be ADDITIVE, not a core patch

> *"its not forbidden, but only use when needed. best if we are just an **extension that can be
> used in already compiled qemu**. but this is only to help ease of deployment, **patching qemu
> doesn't block workloads**. maybe extensions need to be built in, then we ask people to
> recompile qemu. however, know that **patching qemu (core) rather than adding … is fragile when
> qemu updates**, so pls avoid or keep it so it works out of the box as much as possible on qemu
> updates. **same all of this for cloud hypervisor**."*

### Ruling 2 — the SUPPORT-MATRIX ASYMMETRY

> *"qemu/cloud hypervisor is the first to drop support for versions, **its the one where I
> actually accept dropping major versions** for support and ask people to install locally or
> rootless a supported version. **this is not true for kernel version, nvidia version or gpu
> architecture** — there we must be open to support all major versions."*

On what "all major versions" means for the driver axis:

> *"all major ones that nvidia supports for sure. the subset should **at least** contain all
> versions nvidia provides updates for. and the best subset is everything you come across on
> **every serious host** — well-established vast hosts, gcloud/aws machines, or most laptops."*

### Ruling 3 — BASIC `nvidia-smi` is a SHIP MILESTONE

> *"partially side quest and milestone, it is still important to ship. **basic nvidia-smi is a
> milestone (that it even runs and displays running processes)**, fixing all extra info errors is
> a side quest."*

Ruling 3 is applied in `../MILESTONES.md`; it appears here only because it is one of the three
rulings of the same hour and a reader looking for one will look for all three.

---

## 1. The integration ladder — ranked, best first

| rank | shape | what the user does | when it is allowed |
|---|---|---|---|
| **1** | ✅ **An extension loaded into an ALREADY-COMPILED VMM.** The user's existing distro/vendor VMM binary, untouched. | installs a package; points the VMM at it | **always preferred** |
| **2** | ◐ **A compiled-in module the user rebuilds the VMM for.** Additive: a directory we own plus the minimum hunks to register it. | builds the VMM once from a stock checkout + our overlay | when rank 1 is unavailable, **with the reason named** |
| **3** | ⊘ **A patch to the VMM's own core.** Upstream semantics modified. | applies and maintains a patch series | **only when needed, and only with a written justification naming what it buys** |

Rank 3 is *"not forbidden, but only use when needed."* It is not banned; it is **budgeted**. A
rank-3 integration that lands without a written justification naming the specific capability it
buys is a defect in the change, not a matter of taste.

### 1.1 ★★ The acceptance test is NOT "does it work"

> **The acceptance test for a VMM integration is: does it still work after the VMM updates?**

This is the whole reason the ladder is ordered the way it is, and it is the sentence to test a
proposed integration against. Rank 1 survives a VMM update by construction (we are not in the
binary). Rank 2 survives unless our two registration hunks conflict — and a conflict there is a
**build failure**, which is loud. Rank 3 survives only until upstream touches the same code,
and its failure mode is the dangerous one: a mis-rebase changes behaviour without failing to
build.

A green "it works on my VMM build" is therefore **not** evidence about a rank-2 or rank-3
integration. The evidence is a build against a VMM version **we did not pin to**.

### 1.2 ⊘ The one thing that is NOT on the table

> **Deployment ease is the ONLY thing at stake in this ladder.**

⊘ Never trade a correctness or data-plane property to climb it. *"patching qemu doesn't block
workloads"* — the ladder exists to make installation pleasant, and a pleasant installation of a
product that drops guest writes is worth nothing. Concretely: if the only rank-1 shape available
costs us a data-plane property, we take rank 2 and write down why. §3 is a live instance of
exactly that trade, and it was refused.

### 1.3 ⊘ The ladder is per-VMM, and it applies to Cloud Hypervisor identically

*"same all of this for cloud hypervisor."* There is no QEMU-shaped exception. A rank-1 story on
QEMU and a rank-3 story on Cloud Hypervisor is not "we support both".

### 1.4 ★ An UPSTREAMED patch is a rank-1 investment, not a rank-3 cost

A one-time patch we get **merged upstream** is not the same object as a maintained fork. After
one release cycle it is in the distro binary, and our integration is rank 1 forever. This is the
only route by which a rank-3-looking change is worth taking *for deployment reasons* — and it is
worth taking eagerly, because the cost is bounded and paid once.

The test that separates the two: **would upstream take it?** A patch upstream would take is an
investment. A patch upstream would refuse is a fork with a hopeful name.

---

## 2. The support matrix — where a version floor is legitimate

| axis | posture | why |
|---|---|---|
| **VMM** (QEMU, Cloud Hypervisor, …) | ✅ **a version FLOOR is legitimate** | *"its the one where I actually accept dropping major versions … and ask people to install locally or rootless a supported version"* |
| **Guest/host kernel version** | ⊘ **open to ALL major versions** | not ours to choose; a user does not reinstall their kernel for us |
| **NVIDIA driver version** | ⊘ **open to ALL major versions** | see §2.2 for the concrete subset |
| **GPU architecture** | ⊘ **open to ALL major versions** | the whole point of `kayfabe-arch` |

### 2.1 ★★★ What this reverses

> **A version floor is an engineering tool in exactly ONE place.**

The default it replaces is the ordinary engineering instinct that a narrower support range is a
legitimate trade against implementation cost. On three of the four axes above **it is not a
trade at all** — it is a **product defect**, and it should be reported and fixed like one.
"We only support driver 580 because the encoders are const-size" is not a scoping decision; it
is a bug with a rationale attached.

⊘ This is the sentence a future agent will be tempted to soften. Do not. The asymmetry is the
ruling; a symmetric reading of it (*"floors are fine where they buy enough"*) is the position
that was overruled.

### 2.2 The driver-scope definition, concretely

Three nested statements, in increasing ambition. The **floor of the requirement is the second**:

1. *"all major ones that nvidia supports for sure"*
2. **the subset must AT LEAST contain every version NVIDIA still provides updates for** ← the bar
3. *"the best subset is everything you come across on every serious host"* — well-established
   vast hosts, GCloud/AWS machine images, and most laptops ← the target

Statement 2 is a **moving** bar: NVIDIA's supported-branch list changes, so a subset that
satisfied it last year can fail it this year with no change on our side. It is therefore not
something to check once at design time.

⚠ **The live counter-example is ours.** `host_driver_version_pin.md` records the host edge
pinned to `[580.65.06, 581.0.00)` because `kayfabe-isolate-host/src/rm.rs` uses const-size,
version-free encoders (`four_axes_of_variation.md` §1, "Host driver version" row). Under this
ruling that pin is a **known product defect with a named refusal in front of it**, not a scope
boundary — which is the framing that row already uses ("axis still unbuilt — and deliberately
so"), and this record only removes the option of ever calling it "decided".

### 2.3 Where the VMM sits in the axis list

`four_axes_of_variation.md` names four axes and `compatibility_matrix.md` extends them to six
(adding guest kernel version and multi-GPU). **The VMM is on none of those lists.** It is a
further axis, and under Ruling 2 it is the *only* axis of any of them that may carry a floor.
That is the one-line summary worth carrying: **the VMM is the axis you are allowed to narrow,
and the only one.**

### 2.4 ★ What a legitimate floor still owes

A floor is legitimate; a *silent* floor is not. A VMM floor must be:

- **stated as a number**, not as "recent";
- **checked**, and refused **by name** at the earliest point it can be — a build failure if the
  facts are compile-time, a realize-time refusal if they are properties of the running binary;
- **argued from a SEMANTIC fact** — a facility that does not exist below the floor — never from
  "we tested on this one".

We already do this, and it is the model to copy: `kayfabe-qemu-raw`'s crate docs record **two
different floors with two different subjects** — a compile-time `#error` at 9.2 about *symbols*
(every function the shim names is present there) and a realize-time floor of 10.2 about
*semantics* (the global-lock opt-out did not exist before it) — with the observation that on a
9.2 build the device compiles, registers, realizes its C half and is then refused **by name** by
the Rust half. Two floors, two subjects, each refused where it can be seen.

---

## 3. ★★★ Applying Ruling 1 to what we have built — and the ONE open experiment

This section is the ruling's cash value, because our current integration is **rank 2**, and the
rank-1 escape is already named in the design.

**Where we are: rank 2 (compiled-in, user rebuilds).** `l2_qemu_adapter.md` §2.1 establishes it
mechanically at the pinned tag — a QOM type is resolved to a module through `module_info`, a
table generated at QEMU build time (`v10.2.0 util/module.c:319`), so a `.so` outside that table
can never be found by type name; and a module found by path must still carry a per-build stamp
(`:176`, `include/qemu/module.h:18`), whose failure hint upstream prints verbatim as *"Only
modules from the same build can be loaded."* [inferred, from those two source reads] there is no
out-of-tree device mechanism at that tag. DECISION Q1 therefore ships an **additive overlay**:
a directory (`hw/misc/nvkvm/`) plus two hunks (one `meson.build` line, one `Kconfig` stanza).

**That is correctly rank 2, and §2.1 already prices it honestly** — *"We do not get 'install QEMU
from your distro and run'. The user builds QEMU once… the single largest unpaid cost in this
milestone."* Under Ruling 1 that price is **acceptable but not final**: rank 2 is what you take
when rank 1 is unavailable, and you keep looking.

**The rank-1 escape is `vfio-user`, and it is already written down** (`l2_qemu_adapter.md` §9.6).
A vfio-user *server* makes our device a separate process speaking a documented socket protocol to
a **stock, unmodified, distro-packaged** VMM. It deletes §2.1's packaging cost entirely, deletes
the second unsafe crate, and deletes the foreign-lock class from our process. QEMU ships
`hw/vfio-user/` at v10.2.0 with `S: Supported` in MAINTAINERS.

**⊘ And it is refused today for a reason Ruling 1 itself supplies.** §9.6's measured blocker is
that nothing in `hw/vfio-user` marks its regions for the lock-free dispatch path, so a trapped
BAR access becomes a socket round trip taken with the VMM's global lock held — reintroducing the
5.3× amplification `qemu_bql_spike.md` §5 measured and the floor decision was taken to remove.
**That is a data-plane property, and §1.2 forbids trading one for deployment ease.** The ladder
does not override the data plane; it is subordinate to it.

> ### ★ THE EXPERIMENT THAT DECIDES IT — and §1.4 says which answer to want
>
> §9.6 already specifies it: measure a trapped BAR round trip through the vfio-user client
> against the same access through an in-tree lock-free device, **and check whether marking the
> client's regions lock-free is a patch upstream would take.**
>
> Under §1.4 the second question is the one that matters, and a "yes" is worth chasing hard: an
> upstreamed one-line marking converts our whole integration from rank 2 to **rank 1 on both
> VMMs at once** — Cloud Hypervisor is a vfio-user client too, so one server serves both. That
> is the largest single deployment win available to this project, and it is currently gated on
> an experiment nobody has run.
>
> [assumed] Cloud Hypervisor's vfio-user client support has not been read from source in this
> repo. **Read it before quoting the "both VMMs at once" claim as established** — it is the load-
> bearing half of the argument and it is currently an assumption, not a measurement.

---

## 4. Consequences that are now settled, so nobody re-derives them

1. **A QEMU version floor is NOT a contradiction of "no fork".** It is the sanctioned instrument
   (§2). The thing to avoid is a **maintained divergence from upstream**, not a minimum version.
   These two were conflated across four documents; §6 records the fix.
2. **An additive overlay is not a fork**, and the distinction is mechanical rather than
   rhetorical: an overlay's only conflict surface is its registration hunks, and a conflict there
   fails the build. A core patch's conflict surface is upstream's own semantics, and a
   mis-rebase changes behaviour silently. (`l2_qemu_adapter.md` DECISION Q1 makes this argument;
   this record adopts it as the general rule.)
3. **Narrowing kernel / driver / architecture support to simplify an implementation is a defect
   report, not a design option** (§2.1).
4. **"It works" is not the acceptance test for an integration; "it still works after the VMM
   updates" is** (§1.1).
5. **Rank 3 is available** — with a written justification naming what it buys. It is budgeted,
   not banned.

---

## 5. What this record does NOT establish

Stated so the decision is not read as wider than it is.

- ⊘ It does **not** say Cloud Hypervisor is supported, designed for, or scoped. It says the
  ladder and the matrix apply to it identically **when** it is built. See
  `vmm_portability_seam_audit.md` for what a second VMM would actually cost today, measured.
- ⊘ It does **not** set the QEMU floor. That number lives with its argument in
  `l2_qemu_adapter.md` §3.5 and `kayfabe-qemu-raw`'s crate docs; this record only rules that
  *having* one is legitimate.
- ⊘ It does **not** decide vfio-user. §3 records that the deciding experiment is unrun and what
  would make it the better design.
- ⊘ It does **not** claim our current driver/kernel/architecture coverage meets §2.2's bar. It
  rules that failing to meet it is a defect. `compatibility_matrix.md` is where the actual cells
  live.

---

## 6. The contradiction this record closes

Before 2026-08-09 the tree said, in four places, that the QEMU version requirement had been
cancelled *and* that it was the remedy. Both halves were reachable by search, so a reader found
whichever one they searched for first.

The resolution Ruling 1 + Ruling 2 forces is one sentence:

> **A version floor is fine. A maintained fork is what to avoid. They were never the same thing.**

The audited sites and their before/after stances are listed in §6 of
`vmm_portability_seam_audit.md`, and each one now links here.
