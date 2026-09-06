# Project history — where kayfabe came from, and what Mode 1 / Mode 2 mean

> ### STATUS — 2026-09-06 / **LIVE**
>
> Lineage and vocabulary. Moved out of `README.md` on 2026-09-06 as part of the front-page
> rewrite; the text is preserved verbatim from `04860c81` except for added headings and the
> dated corrections marked as such.
>
> For *current* state read [`STATUS_DETAIL.md`](STATUS_DETAIL.md); for the full technical
> account read [`whitepaper/kayfabe_architecture.pdf`](whitepaper/kayfabe_architecture.pdf).

---

## 1. What the project does

Kayfabe **emulates an NVIDIA GPU inside QEMU**. The guest is handed what looks like real
hardware — an emulated device plus a faked GSP — and runs the **real, unmodified NVIDIA
driver** against it. Nothing in the guest is patched, shimmed or replaced; it does not
know it is virtualised. We recover what the guest is actually trying to compute from its
own protocol (RM allocations, page-directory binds, doorbells, pushbuffer methods) and
forward that work to a real GPU on the host.

Two things follow, and they are the whole point:

- **Nothing in the guest has to be modified**, so the guest OS stops being a constraint.
  Linux today; **Windows guests are the end goal** — you cannot ask a Windows guest to
  load your custom kernel driver, but you can let it load NVIDIA's own.
- **The guest's driver is decoupled from the host's.** Because the guest talks to an
  emulated device rather than to the host driver, the two versions do not have to agree
  and are free to drift.

The target is **parity performance** — the forwarding should cost approximately nothing
against running on the host directly.

## 2. Mode 1 and Mode 2

The two designs this project has tried, and why the names appear everywhere:

- **Mode 1** — forward the guest's ioctls to a real NVIDIA device on the host. The guest
  runs a **custom kernel driver** that knows where to send them. Proven and shipping.
- **Mode 2** — emulate the device itself, so the guest's **stock kernel driver** works
  unmodified. Harder, and what this repo is.

**kayfabe** is a clean-slate, **Mode-2-only** rewrite of the original `nvkvm` C prototype,
in Rust: hypervisor-agnostic, multi-tenant, unprivileged per-guest-process host isolates.
The thesis is multi-tenancy — several guest processes, several guests and several GPUs
sharing one host GPU with per-process blast-radius containment, which the C artifact
proved feasible and this rewrite is meant to make structural.

## 3. Relationship to nvkvm-pv

**Coming from [nvkvm-pv](https://github.com/reindertpelsma/nvkvm-pv)?** It is the
maintained Mode-1 stack. Different design, different trade:

| | nvkvm-pv (Mode 1) | kayfabe (Mode 2) |
|---|---|---|
| guest **kernel** driver | **custom** — a module you build and load in the guest | **stock NVIDIA**, unmodified |
| guest **userspace** | stock NVIDIA libraries | stock NVIDIA libraries |
| guest ↔ host driver versions | must **match** — guest userspace is installed to match the host | **decoupled by design**; free to drift |
| guest OS | Linux — it needs the module | any, in principle; **Windows is the end goal** |
| status | **works today**, tested, maintained | **research in progress** |
| language | C | Rust |

If you want GPU forwarding that works today, use nvkvm-pv. Kayfabe is the bet that Mode 2
buys the unmodified guest — and with it Windows, and multi-tenancy — that Mode 1 cannot.

## 4. The C research prototype

`archive/nvkvm/` is the original **nvkvm** C research prototype, snapshotted at commit
`bac00b6` and imported as a single squashed commit. It is the artifact kayfabe was
rewritten from, and it is the only implementation a real NVIDIA driver has ever accepted
end to end on the Mode-2 path. It is kept as a **differential oracle**, not as code that
runs here — see [`../archive/README.md`](../archive/README.md) for exactly what it is and is
not.

Where a kayfabe design doc says *"the C artifact does X"*, `archive/nvkvm/` is the code it
means. ⚠ The C is a **single-process** Mode-2 oracle, it is an oracle for RM semantics and
control-plane ordering rather than for engine execution or throughput, and its own
`CLAUDE.md` carries the scoping corrections — read those before citing it.

## 5. Corrections log — claims the README carried and later had to withdraw

Kept because the house rule is that a correction is relocated, never deleted, and because
the *pattern* is more instructive than any single entry: every one of these was a true
sentence that stopped being true and did not say so.

| dated | the claim | what replaced it |
|---|---|---|
| 2026-07-27 | *"283 tests"* | a literal count rots within the week; `cargo test --workspace` is the count of record |
| 2026-07-27 | *"four CI gates"* | six jobs; `.github/workflows/ci.yml` is the list of record |
| 2026-07-27 | mutation score **99.2 %** L0 / **92.44 %** L1 | not quotable — the gate's scope changed to every production crate and the 91 % floor is pending re-derivation |
| 2026-07-27 | *"the two `KAYFABE_SLOW`-gated tests"* | membership has grown; the `skip_slow!` call sites are the list of record |
| 2026-07-27 | `kayfabe-abi` is *"a stub, trait shape only"* | built — offline generator, generated `#[repr(C)]` structs, version-dispatch decode, oracle tests |
| 2026-07-28 | `kayfabe-gsp` is *"unverified / under construction"* | built (S0–S5, 8 modules, ~3,550 L); genuinely unbuilt there is reboot/resume S6–S8 and the bridge to the core |
| 2026-09-06 | *"1554 pass, 1 fails" at `06bbfd9e`* | the shape of that number was the tell — it was a truncated run, not a whole-suite measurement |
| 2026-09-06 | the 9 failures are *"a bookkeeping gap, not a functional defect"* | at least 6 of 9 are functional/safety/structural, two of them completion forgery; see [`STATUS_DETAIL.md`](STATUS_DETAIL.md) §2 |
| 2026-09-06 | *"no real adapter (Linux, QEMU, or NVIDIA arch) exists yet"* | false of the first two axes — `kayfabe-vmm-kvm`, `kayfabe-vmm-qemu`/`-qemu-raw` and `kayfabe-isolate-host` are real adapters; true only of the NVIDIA-arch axis |
| 2026-09-06 | *"zero unsafe blocks anywhere"* | two audited crates relax the workspace `forbid`: `kayfabe-linux-raw` (86 blocks) and `kayfabe-qemu-raw` (29), both gated by naming, `// SAFETY:` and a CI ratchet |
