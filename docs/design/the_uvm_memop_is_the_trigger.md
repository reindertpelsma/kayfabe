# The UVM MEM_OP is the trigger, and we throw it away

**STATUS: LIVE, 2026-09-11.** Owner correction to `RESUME_HERE_w417.md`'s w426 section, which
concluded *"there is no trigger to hold at"*. That conclusion was wrong.

> **Owner, 2026-09-11:** *"uvm uses mem ops that we can block as invalidate point thats
> synchronization point 3. 3 points that can refresh pdb/pde/ptb/pte: tlb invalidate (no),
> rm map (no), uvm kernel channel (yes)"*

## The chain, end to end — every link exists except the last

    ogkm-580  uvm_mmu.c:722 :806 :1210 :1335   host_hal->tlb_invalidate_all(&push, pdb, …)
    ogkm-580  uvm_ampere_host.c:45-70          NVC56F_MEM_OP_A/_D, OPERATION=MMU_TLB_INVALIDATE
    ours      kayfabe-chips/ga10x.rs:1450-65   decodes both variants, extracts `pdb` + `membar`
    ours      kayfabe-fwd/lib.rs:8166-8169     PushMethod::TlbInvalidate => out.invalidates.push
    ours      ——————————————————————————       NOTHING READS `out.invalidates`

⊘ Census over `crates/` and `tests/`: the identifier appears at its declaration
(`kayfabe-fwd/src/lib.rs:6069`), at that one push (`:8167`), and **otherwise only inside four
TEST files**. No production reader, no destructuring of the outcome type anywhere.

⚠ **Four tests assert its contents**, so it is green in CI while inert in production — *"a green
test can hold a wall in place"*, and the reason a census over PRODUCTION readers is a different
question from a census over mentions.

## Where it is dropped is where it was needed

`kayfabe-rt/src/device.rs:3500`:

```rust
let parsed = self.parse_pushbuffer(vmm, pid, cid, &fresh)?;
let spans = parsed.ce_spans.len();
if !parsed.ce_spans.is_empty() {
    let fwd = self.forward_ce(pid, cid, &parsed.ce_spans)?;   // <-- the forward
```

The caller reads `ce_spans`, ignores `invalidates`, and **forwards two lines later**. The guest
has just told us its page tables changed, and we hand its work to the host without acting on it.

## ⇒ The fix, and why it needs the deferred doorbell

1. **Consume `out.invalidates`**: refresh the page tables for those PDBs.
2. **Before the forward** — the C's own invariant, `nvkvm_gpu_emul.c:582`: *"Fault-safe: a
   mapping is always backed before the engine that uses it runs."*
3. **Not on a vCPU.** `ring_inline` runs on the vCPU, and a refresh there is a blocking trap —
   *"a trap may not take longer than a millisecond"*. So the forward must move to the worker,
   which is the deferred doorbell lane (`DoorbellAsyncArm::defers()`), today default-off.

⇒ That is exactly the architecture the owner has stated twice: *"MMIO writes primarily touch a
queue to register that a write happened there and wake a coordinator… Doorbell write as well"*,
and *"a doorbell may only queue+wake or forward"*. The doorbell keeps queue+wake; the worker
does parse → refresh-if-invalidate → forward.

⊘ **It does not contradict the doorbell-is-a-hint ruling.** We are not blocking a doorbell
hoping to win a race: we control when the HOST sees the work, because we are the ones who
forward it. Ordering our own forward after our own refresh needs no guarantee from the guest.

## ⚠ What is still unmeasured

Whether that method REACHES our decoder for the UVM kernel channels at runtime, or whether
those channels are passed through unparsed. `memop_census::note` is called on the same line and
is the instrument. The bench box was destroyed by its own deadman during the handover;
`50579731` is loading.
