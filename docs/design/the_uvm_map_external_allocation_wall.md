# The `UVM_MAP_EXTERNAL_ALLOCATION` wall

**STATUS: LIVE, 2026-09-13 (w695i/w695j).** Measured, not inferred. Supersedes every earlier
account of the `cuCtxCreate` wall in this campaign — in particular the w694 reading
(*"the projection never files cuCtxCreate's channels"*), which is **refuted**: see §4.

## 1. Where the guest actually stops

`[measured w695i]`, cup3 traced from exec under a 60 s budget:

```
20:38:07.397  ioctl(9, _IOC(_IOC_NONE, 0, 0x21, 0) <unfinished ...>
20:38:56.074  --- SIGTERM ---            <- 49 SECONDS, never returned
              state=R   utime/stime = 7 / 13820   (138 s of SYSTEM time)
```

UVM ioctl `0x21` is **`UVM_MAP_EXTERNAL_ALLOCATION`** (`ogkm-580 uvm_ioctl.h`). ⊘ It is not a
userspace spin and not a JIT-cache loop: nvidia-uvm is **inside the ioctl**, burning kernel CPU.

★★★ This is the wall the research artifact's live oracle already recorded, from the other side
(`CLAUDE.md`, the `nvdiff` section): *"the guest runs in lockstep with hardware to
`UVM_MAP_EXTERNAL_ALLOCATION` — 221 of `cuCtxCreate`'s 479 ioctls (46.1 %) — then calls
`0x20801702` ×175 until killed. Hardware calls that id **zero times**."*

`0x20801702` = `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS` (`ogkm-580 ctrl2080mc.h:176`) — UVM asking
RM to drain the interrupt path, over and over, **because it is waiting for a completion that
never comes**. Two independent instruments, two codebases, one wall.

## 2. Why the completion never comes

`[measured w695j]`:

```
VAS-REFRESH-SPLIT pdb=0x2efa9c000 backed=0 refused=110 [resolve=0 pin=110]
VAS-ROW-REFUSED #1 at=pin_guest_ram va=0x420000000 gpa=0x10d15e000
                   len=0x1000 len&0xfff=0x0 err=SystemDataPlane
```

- `resolve=0 pin=110` — the VMM resolves every row. **Every** refusal is the pin.
- `len & 0xfff == 0` on every row, each exactly one page.
- `err=SystemDataPlane` — `plan_pin_guest_ram` refuses proc 0 **by name**.

⇒ UVM's page-table rows belong to the **SYSTEM proc**, and §12.26 rules that the system proc has
no data plane: its work is *"forged … never forwarded, so the system proc never mints host
memory."* `0 forwarded (host channel rung)` across the whole boot is the same fact from the other
side.

## 3. ⊘ What this measurement KILLED

- **The 64 KiB rounding hypothesis is dead for these rows.** The standing lead — the C rounds
  every promote-derived mapping up to 64 KiB (`C: src/qemu/nvkvm_gpu_emul.c:7920`) while this port
  binds at the declared length — predicts non-page-aligned rows. All 110 are `len=0x1000`,
  `len & 0xfff = 0`. ★ The criterion was written into the log line **before** the boot, so the
  same line answers it in either direction.
- **`resolve_guest_ram` is not involved.** Zero of 110.
- **The doorbell lane is not losing submissions.** `[measured w695m]`
  `arrived=50 served=44 refused=0 ⇒ UNACCOUNTED=6 | coalesced=6 depth_at_teardown=0 ⇒
  unexplained=0`. The residue is **entirely coalescing** — a doorbell merged onto a pending
  token owes no second completion — and the queue drains to empty. ⊘ w695l published that
  residue as *"submissions that entered the queue and never came out"*; it was not measured, and
  the boot after the criterion was joined to the number refuted it. Nothing is lost here.

## 4. ⊘ And what this campaign believed that was wrong

Each of these was measured, believed, and then refuted **by a later measurement in the same
session** — the sequence is worth keeping, because every one of them looked like the answer:

| believed | refuted by |
|---|---|
| the projection drops cuCtxCreate's channels (w694) | `dropped_no_gpu=0` on all 21 samples (w695b) |
| 482 guest submissions are refused | 425 of them were **our own worker** ringing a GSP sequence number (w695d) |
| the interrupt path is broken | `486 vectors delivered`, the 30 refusals being the pre-enable window (w695e) |
| a refused alloc blocks `cuCtxCreate` | all 16 refusals are the guest KERNEL's clients and the RC watchdog (w695e) |
| libcuda spins in userspace on a semaphore | 88 % ioctl; it is blocked **in** a UVM ioctl (w695f/w695i) |

⚠ Three of the five were instrument defects, not device defects. Two of those three were **one
counter covering two or more causes** — the class this tree already names, encountered three
times in a single session.

## 4b. ⊘ It is NOT established as a regression — the good end fails too

`[measured w698, 2026-09-13]` Two hypotheses were tested and both died:

1. **"The always-on whole-VAS sweep (w533) broke it."** `KAYFABE_PT_SWEEP=off` had returned
   `CUP3_VAL=43` twice, and w533 (2026-09-12) deleted the disarm six days after the known-good.
   ⇒ Restored the disarm on a branch and booted: **cup3 hangs identically with the sweep off.**
   The sweep is exonerated.
2. **"It regressed since 2026-09-06."** Booted `0764a990` — the known-good commit — with today's
   harness overlaid so only the device varied:

       FAIL cuCtxCreate(&ctx,0,d) -> unspecified launch failure (719)

   ⊘⊘ **The known-good does not reproduce on this bench.** `CUP3_VAL=43` was measured on a
   DIFFERENT machine (vast 50013922, the fifth machine). Here, that same commit fails at
   `cuCtxCreate` too.

⇒ A `git bisect` over the 576 commits in range would have bisected noise. ★ **Boot the GOOD end
first**; a bisect that never verifies its good end is measuring nothing.

⚠ Confounder not excluded: this box's `guest.qcow2` has taken many boots including an abrupt
`pkill`. Guest-side state is a live alternative to host/GPU differences, and a fresh guest would
separate them.

★ One real change survives: old code fails FAST with a named GPU error; today's HANGS indefinitely
burning kernel CPU. That is a regression in **debuggability** even where neither configuration
passes.

## 5. The open question, stated as a decision

UVM's page-table work is **guest-kernel work** (so it lands in proc 0) that **must actually
happen** for a user process to run. The standing rule forges system-proc work rather than
forwarding it. So one of these has to give:

1. **The rule's scope is wrong for UVM** — UVM's VAS should not be the system proc, or
2. **The work must genuinely execute** — we write the page tables ourselves rather than forging a
   completion.

⊘ `SystemDataPlane` is **not** a defect and must not be "fixed" by deleting the refusal; it is a
standing design rule with a security argument behind it. The question is whether its **scope** is
right here.

★ Note what the C did, because it bears directly: it mirrored the guest's page tables
**wholesale** and committed the mirror **before any completion became observable**, and went green
*"without servicing or forwarding a single GPU fault"* (`CLAUDE.md`). That is option 2, and it is
the only shape a real driver has ever accepted end-to-end.
