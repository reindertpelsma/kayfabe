# §5.10 — THE ISOLATE'S OWN ADDRESS SPACE: separation, not a reservation

**Status:** landed and measured on a real GA106 (`vh`, GA106 / 580.159.04), 2026-08-10.
Revisions: falsifier `124b69b`, instrument corrections `83651d8` and `cc5d55c`, fix `254cf38`,
pool fix `b66bd44`, boot evidence `1a5d16e`.

> **The owner's invariant:** *VMM state must never be placed where a guest VA can name it.*

---

## 0. ★★★ LEAD WITH THE REFUTATIONS — including of the brief that ordered this rung

- ⊘ **"It is not exploitable today"** — the brief and the audit (`C: s1_what_does_it_protect.md`
  §3) both scoped the defect as *latent*, on the argument that nothing copies guest commands
  into a host ring yet. **That argument is about GUEST-AUTHORED methods and it does not bound
  the hazard.** The address was reachable from *any* channel bound to that space, and this rung
  built one out of the ordinary production verb (`alloc_channel_on`) in ~40 lines. The audit's
  own row said the reachability question was `[NOT MEASURED]`; it is now measured, and the
  answer is that the only thing standing between a guest and the isolate's semaphore was that
  nobody had pointed an engine at it.
- ⊘ **"A host-private reservation is the real fix"** (`rm.rs`'s own doc comment, standing since
  the channel work). **A reservation is a different fix for a different problem.** It stops
  RM's *allocator* from colliding with our objects; it does nothing about a guest **naming**
  them, because the mapping is still in the page tables the guest's engine walks. Both fixes
  are one sentence long and they are not the same sentence.
- ⊘ **My own first two instruments answered their own question**, and one of them **inverted
  the verdict**. See §4. The rung's most transferable output is not the fix.
- ⊘ **"Only OUR objects move"** (the brief) is right about the guest's VAs and incomplete about
  the cost: the isolate's engine still has to resolve **guest** VAs, so every publish is now
  placed **twice**, at one address. Nothing the guest names moved; the number of mappings did.
- ⊘ **This rung does not move the guest**, and no part of it was supposed to. `cup2` is
  unchanged, `forwarded` is unchanged, the missing method-copy is still missing.

---

## 1. What was true before, measured rather than argued

`plan_ce` → `ce_channel(vas)` → `alloc_channel_on(vas, COPY0)` → `raw_map_dma` put the
isolate's **ring, USERD and completion semaphore** into `Vas::host_vas` — the one address
space a guest channel is bound to, alongside every fabricated publish, every guest-RAM pin and
w228's FB leaves. The address was RM-chosen, which makes it **unpredictable, not unnameable**.

The invariant rested on one sentence, in a doc comment, gated by nothing:

> *"…memory the isolate allocated for itself, **which no guest ever names**."* — `rm.rs`

**At `cc5d55c`, on a real GA106** (`kayfabe-rm-ladder --gpu 0 --executor-vas-alias`):

```
??  R30 spaces      = guest range 0xcafe0005, control range 0xcafe0005 — ★ THE SAME SPACE
FAIL R30 arm A      = the isolate's ring at 0x1_20020000 (semaphore 0x1_20022000) is
                      ALREADY MAPPED in the space a guest channel is bound to
FAIL R30 arm C      = a copy engine BOUND TO THE GUEST'S SPACE retired a read of
                      0x1_20022000 and moved 0x00000001. Our last payload was 0x00000001
```

★ The value is the discriminator. `0x00000001` is the payload **the isolate's own last copy
released into that word**, and the probe channel — whose ring and both operands are placed at
dictated addresses three objects apart at `0x7_0000_0000` — has no other way to obtain it.

---

## 2. The fix: SEPARATION

`ExecutorVas` is a **different `FERMI_VASPACE_A`**, one per guest `Vas`, holding the isolate's
own control structures. No guest channel is ever bound to it.

```
        guest-facing space                    executor space  (ExecutorVas)
        ─────────────────                     ──────────────
  the guest's materialized channel      the isolate's CE ring / USERD / semaphore
  fabricated publishes      ── at the same VA ──▶  fabricated publishes
  guest-RAM pins            ── at the same VA ──▶  guest-RAM pins
  FB leaves                 ── at the same VA ──▶  FB leaves
```

- **Guest VAs do not move.** `#102` holds unchanged: a publish is still placed FIXED at the
  guest's own VA in the guest-facing space. `map_dma_both` then places the same object at
  **the address RM reported back**, in the executor space. The guest-facing placement is the
  authority and the shadow is derived from it — ⊘ the reverse order would let a shadow refusal
  relocate a guest VA, which is `#102` broken by the fix meant to protect it.
- **Why the operands must be dual-mapped at all:** the isolate's copy engine is now in a space
  the guest's addresses do not live in, and the operands it is asked to copy **are** guest VAs.
  A publish that reaches `raw_map_dma` directly lands in one space only, and the failure is an
  `Xid 31 FAULT_PDE` on a later copy — nowhere near the omission.
- **Per guest `Vas`, not one per isolate** — `#14`. Two procs publish at identical guest VAs by
  construction, so a single shared executor space would collide on the first fixed map.

**At `254cf38`, same command, same box:**

```
ok  R30 spaces      = guest range 0xcafe0005, control range 0xcafe0009 — two spaces
ok  R30 arm A       = the isolate's 65536-byte ring object at 0x1_20020000 is UNCLAIMED
                      in the guest-bound space
ok  R30 arm B       = the SAME call REFUSES in the control space (NoMemory)
★   R30 arm C       = the guest-bound engine did NOT retire a read of 0x1_20022000
```

and the host `dmesg` names it, which is the strongest single line this rung produced:

```
NVRM: Xid (PCI:0000:00:07): 31, name=kayfabe-rm-ladd, channel 0x00000005,
      MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x1_20022000.
      Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
```

⇒ `FAULT_PDE` at exactly `sem_va`: **hardware's own word that the address has no page
directory entry in the space a guest channel is bound to.** ⚠ That Xid is the boundary
working. Arm C is opt-in (`--executor-vas-alias`) for exactly that reason.

---

## 3. Structural, not conventional — because a doc comment is what we had

| half | what it pins | where |
|---|---|---|
| **type** | `ExecutorVas` cannot be *named* outside the crate: private field (E0451), no tuple ctor (E0423), no `From<HostHandle>`. `ce_channel` and `alloc_channel_for_isolate` take one, so a caller holding a guest `Vas` **cannot spell the argument** | `crates/kayfabe-isolate-host/tests/ui/{name,call}_an_executor_vas.rs` |
| **census** | the half a type cannot see: Rust's privacy unit is the crate, so *inside* it the struct expression is spellable. 5 rulings on the mint surface, plus a no-binary-mints check and an ordering check that `map_dma_both` maps the guest side first | `crates/kayfabe-isolate-host/tests/executor_vas_census.rs` |

★ Two `trybuild` rows, not one, because rustc reports **only the first** error when both
spellings live in one file — a single-row suite pins one spelling and silently stops checking
the other. (Found by writing it as one file and reading the blessed stderr.)

---

## 4. ★★★★★ THE INSTRUMENT FAILURES — three, and the third inverted the verdict

Every one was found by *watching an arm that was supposed to turn green fail to*. None would
have been found by reading the code.

1. **The probe's own object alignment answered the question.** `probe_va` allocated a 64 KiB
   object and asked for it at `ring_va + 0x2000`; `alloc_device_local` passes
   `alignment = len`, so RM rounded the mapping down to `ring_va` and reported a different
   address. That reads **exactly** like *"occupied, and the fixed ask was a hint"*.
2. **…and shrinking the probe did not fix it, which is the more useful half.** A 4 KiB object
   at the same address was **also** placed at `ring_va`: RM maps device-local memory with
   64 KiB big pages, so the allocator cannot be asked a finer question than *"is this 64 KiB
   region taken"*. ⊘ A finer instrument that cannot be finer is **worse than a coarse one** —
   it looks like it resolved something. ⇒ The rung asks about the **ring object, at its own
   base and its own size**. The semaphore is at `+0x2000` inside it, so *"the object is not
   mapped here"* is strictly **stronger** than *"the word is not mapped here"*.
3. ★★★ **The probe's own channel ring landed on the address it was asking about.** Arm C let
   RM choose where its stand-in guest channel's ring went. RM chose `0x1_20020000` — the
   address the isolate's ring had just been **freed from** — so `sem_va` fell **inside the
   probe's own ring**. The copy retired, moved `0x00000000`, and the rung printed *"the address
   RESOLVED in the guest's space — which is already the violation"*.
   ⊘ It was the instrument's memory, not the isolate's. **A correct boundary was being scored
   as a violation.** The tell was in the output and was reported: the value was **not our
   payload**. A two-valued arm would have had nowhere to put that.
   ⇒ Every address arm C owns is now **dictated** — the ring via `alloc_channel_at` (which is
   what R26 exists to establish), both operands via fixed `map_dma`, each **refused rather than
   adopted** if RM disagrees.

★ **The rule:** a probe that allocates from the same allocator, in the same space, at the same
moment, is not an independent observer.

---

## 5. ★★★★ AND THE FIX ITSELF HAD ONE — a POOL SLOT MAY NOT OWN AN ADDRESS SPACE

The executor-VAS table lived in `HostRmBackend` for one revision. `tests/e6_hw_join.rs` caught
it on real hardware and nothing else did:

```
CE-SUBMIT dst=0x200100000 … sem=0x00000001 want=0x00000001 → RETIRED
CE-SUBMIT dst=0x201100000 … sem=0x00000000 want=0x00000002 → NEVER-RETIRED
```

An isolate is a **bounded pool of workers**. A publish and the copy that reads it are two
requests and need not land on the same slot, so a per-worker table gave the second worker a
**fresh, empty** shadow: the operands were mapped in worker A's space and the engine walked
worker B's.

⇒ The table is keyed by an object that belongs to the **isolate** (a `Vas`), so it belongs on
the `RmConnection`, with the other object state. **A pool slot may own a *channel*; it may not
own an *address space* that other slots' mappings are placed into.**

⚠ The mint is three ioctls and cannot be held under the handle lock, so two workers can race.
`remember_exec_vas` is a single critical section that reports **the winner**, and the loser
frees what it built — an address space nothing can name is exactly the orphan
`alloc_vaspace`'s own error arm exists to avoid.

⊘ Note what did *not* catch this: `cargo clippy --workspace --all-targets`, the whole non-GPU
suite, R17, R25, R26, R29 and R30 were all green while it was broken. Every one of those runs
one worker.

---

## 6. What this does NOT establish

- ⊘ **Nothing about the guest's own materialized channel ring.** That is isolate-allocated
  memory which stays in the guest's space **by design** — it is the guest's channel. Scope is
  the isolate's own copy-engine control structures.
- ⊘ **Nothing about the guest.** `cup2` does not pass, `forwarded` does not move, and the
  missing method-copy (`gr_execution_boundary.md` §0.1) is untouched.
- ⊘ **`probe_va` answers the allocator's question at 64 KiB granularity and nothing finer.**
  Arm C is what asks hardware, and it asks about exactly one address.
- ⊘ **Not the cap-dropped case.** `euid` is printed; these runs were root.
- ⚠ **The target's lifetime is shorter than the current code implies.** Once a forwarded user
  channel's completion becomes the guest's own word at the guest's own VA,
  `SEMAPHORE_OFFSET` matters only for the isolate's own copy-engine work. This separates a
  smaller set than `rm.rs` makes it look.

## 6.5 ★★★ THE BOOT — and TWO harness omissions of mine before it meant anything

Three boots at `b66bd44`, archive and QEMU both stamped `kayfabe-rev:b66bd441a6…`:

| tag | plane | FB backing | doorbell census | GR-FB-BACKING |
|---|---|---|---|---|
| `w229a_b66bd44_execvas` | ⊘ **stillborn** | off | 191 / 183 / 8 | 0 |
| `w229b_b66bd44_execvas_real` | real | ⊘ off | 191 / 183 / 8 | 0 |
| **`w229c_b66bd44_execvas_fbback`** | **real** | **on** | **191 / 183 / 8** | **32** |

⚠ **The first two boots could not have shown a regression in this rung, and both looked
completely healthy.** `boot_capture.sh` alone leaves `KAYFABE_ISOLATES` unset, so `w229a` ran
`isolate_plane=stillborn` — no isolate child, no `RmBackend`, none of the changed code. `w229b`
fixed that and still left `KAYFABE_FB_BACKING` unset, so nothing reached `map_gpu_va` and the
census rows read `Framebuffer { .. }` with no backing. Both produced a full `dmesg`, a full
serial log, a printed census and `CAPTURE_RC=0`.
⇒ ★ **Ask which LINE you expect the boot to execute, and grep the log for it.** `w229a`/`w229b`
are kept as the arming controls they accidentally are — the three rows differ by exactly the two
flags.

**`w229c` is the regression test.** The production publish verb, dual-mapping through
`map_dma_both`, on a real GA106:

```
GR-FB-BACKING proc=2 chan=0 SET_VALID_SPAN_OVERFLOW_AREA leaf va=0x200000000 len=0x200000
    fb_phys=0x400000 → BACKED memory=0xcafe005e host_va=0x200000000 placed_as_asked=true
… ×3 leaves, `isolates: 2 materialized, 2 live, 0 refusing`, 0 PlacementRefused
doorbells: 191 arrived, 183 served, 8 REFUSED by name; last token 0x00010001
```

⇒ **`placed_as_asked=true` at the guest's own VA, and the census is byte-identical to `w228`'s.**
The guest's addresses did not move and the guest's behaviour did not change — which is exactly
what this rung claims and the whole of what it claims.

## 7. How to re-run it

```sh
# the falsifier, all three arms (arm C provokes a real Xid 31 when the boundary HOLDS)
./target/release/kayfabe-rm-ladder --gpu 0 --executor-vas-alias ; echo rc=$?
# arms A and B only
./target/release/kayfabe-rm-ladder --gpu 0 --executor-vas
# the regression bar: the CE round-trip and the dictated ring
./target/release/kayfabe-rm-ladder --gpu 0                  # R15 SEM LANDED, R17 CE COPY
./target/release/kayfabe-rm-ladder --gpu 0 --dictated-ring
cargo test -p kayfabe-tests --test e6_hw_join               # ★ the multi-worker one
cargo test -p kayfabe-isolate-host --test compile_fail --test executor_vas_census
```
