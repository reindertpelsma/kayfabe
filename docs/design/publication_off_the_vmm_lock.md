# Why publication does not run under the VMM's global lock

**STATUS: LIVE (2026-08-14).** The ruling `kayfabe-device`'s `pubqueue` is the mechanism for.

⊘ The quote below lives here rather than in the crate because it names QEMU's lock by its
QEMU name, and the portable crates are gated against naming VMM-specific machinery
(`l2_qemu_adapter.md` §2.2 — the gate's own remedy is *"the `Vmm` trait method that means
it"*). ⚠ An owner's words are not reworded to satisfy a lint; they are moved to where they
are allowed. The crate refers to this file.

## The ruling, verbatim

> **Owner, 2026-08-14:** *"10 ms BQL lock seems already bad to me. … why do you need to
> publish under BQL? I don't see a reason. you already have other boundaries to determine
> when to update a pte write — look at the C."*

## What it means in portable terms

Every guest MMIO write arrives with the VMM's global lock held, so any host round trip taken
inline in a trap stalls **every vCPU and the VMM's dispatch loop**, not just the ringing one.
`[w317]` measured a 3.70 s teardown disposal in exactly that position.

⇒ The trap-side call must do **no host I/O**: it queues, and a worker performs the publication.
The census counts how many doorbells still ran inline, and driving that to zero is what
*"publication is off the lock"* means, measured rather than claimed.

★ The second half of the ruling — *"you already have other boundaries"* — is the one that took
longest to act on. Those boundaries are the three synchronization points
(`the_three_synchronization_points`), not the doorbell; see
[the doorbell is not a barrier](the_wait_kind_ruling.md) for why a doorbell cannot be one.
