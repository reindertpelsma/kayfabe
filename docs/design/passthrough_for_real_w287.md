# Passthrough for real — the cursor plane CLOSED, and the operand plane is the wall

**STATUS: LIVE — 2026-08-13 (w287).** Supersedes `ce_passthrough_is_already_built.md` §1 and §3
(w284), whose "one number" blocker this rung removed and whose Option A it built. The three
options in that doc's §3 are **retired**: Option A shipped, and it needed neither Option B's
reverse-lookup-by-phys nor Option C's forbidden widening.

⚠ Everything below is read out of two boots of this branch on a real GA106 (`vh`,
`580.159.04`) plus one bare-metal native arm, and out of the code at `9595b54`.

---

## 0. ⊘⊘⊘ LEAD: THE BRIEF'S BASE CANNOT HOST THE RUNG IT BRIEFS

The rung was briefed to branch from `origin/w286-channel-privilege-census` (`71fbd59`). **Every
code citation in the brief resolves at `b0d6de7` (w284's tip) and none at `71fbd59`:**

| brief says | at `71fbd59` |
|---|---|
| `rm.rs:4140-4176` — `RingOwner::Ours` / `UserdOwner::Ours` | `UserdOwner` **does not exist** (0 occurrences workspace-wide); those lines are the `RingOwner` block |
| `rm.rs:4876` — `alloc_channel_in` | that line is `userd_store_u32` |
| `fwd/lib.rs:5598` — `CeExecutor::Ours` | different code; the symbol is at `:5542` |
| `fwd/lib.rs:5540` — `Representability::Fabricated` | different code |

`71fbd59` is **two commits** off master (`e758778`/w260) and touches **zero Rust** — it is
`docs/design/channel_alloc_forwardability.md` plus `scripts/rpc_channel_census.py`. `b0d6de7` is
**136 commits** off the same merge-base. The entire w261–w284 line — leg A2, leg B,
`UserdOwner`, `AdoptedGuestUserd`, every w263–w283 trace — is **absent** from the assigned base.

⇒ On that base most of this rung looks unbuilt and is not. This branch is `b0d6de7` with
`71fbd59` and the w287 gate lane merged in (both docs-only, no conflicts).

⚠ **And the shared checkout bit.** Two lanes both used `/workspace/kayfabe_w287` on `vh`; one
re-cloned it mid-rung under the other and **a stale binary silently ran**. Lane-private
`REPO`/`CARGO_TARGET_DIR` now carry the lane name, and the client binary is **deleted before
the build** — it carries no `kayfabe-rev:` stamp (measured: `strings | grep -c` = 0), so
existence is the only guarantee available and "no build ⇒ no file ⇒ no run" is how it is had.

---

## 1. ★★★★★ THE HEADLINE — HARDWARE ADVANCED THE GUEST'S OWN USERD CURSOR

This is the sentence the campaign has been unable to write, stated as the blocker one rung ago:
*"the host channel executing the work cannot advance the guest's own USERD cursor."*

`[measured 2026-08-13, boot `w287_guest`, real GA106, HEAD 9595b544]`

```text
GR-BIRTH iso2/gpu0 #1 engine=Ce vas=0xcafe0005 adopt=GUEST-RING memory=0xcafe0006
  ring_va=0x120020000 gp_fifo_va=0x120021000 entries=64
  userd_memory=0xcafe0006 userd_offset=0x3000 userd=GUEST-USERD
```

- `userd_memory` **==** `memory` — **one object**, the guest driver's own shape.
- `userd_offset=0x3000` — this rung's `USERD_OFFSET_IN_RING`.
- `userd=GUEST-USERD` — **leg B fired for the raw client for the first time.**

and the client's own verdict, from inside the guest:

```text
FAIL  R33 arm 1 COPY = dst[0] 0x3f0011cc -> 0x3f0011cc (want 0xc0ffee33),
      semaphore 0x00000000 (want 0x00000001), GP_GET 1 GP_PUT 1
      — the entry WAS fetched and the methods did nothing
```

**`GP_GET 1`.** Three prior boots (`w283`, `w283c`, `w283d`) printed `GP_GET 0 GP_PUT 1`,
byte-identical. ⇒ **The GPU host unit fetched the guest's GPFIFO entry and wrote the guest's own
USERD cursor.** With `CE-SUBMIT = 0` in the same boot, nothing of ours drove that channel.

### ⊘ Why the cursor cannot be an artefact of the change

- `USERD_GP_GET = 34*4 = 0x88`, so the word read is ring `+0x3088`. The ring object's other
  tenants are pushbuffer `0x0..0x1000`, GPFIFO `0x1000..0x1200`, semaphore `0x2000`. **Nothing
  this client writes touches `0x3088`.**
- There is **not one store to `GP_GET` anywhere in `kayfabe-isolate-host`** — the value can only
  have been written by hardware.
- RM **zeroes 512 bytes of a handed-in USERD inside the alloc** (`kernel_fifo_gm107.c:797-808`),
  so the pre-state is a measured `0`, not an assumption.
- The **native arm ran from the same binary minutes earlier** and reached `GP_GET 1 caught
  GP_PUT 1` **with the copy succeeding** — so the layout is validated against bare metal, and
  the guest arm's difference is isolated to what the methods did, not to where the cursor is.

### The ownership table, as HEAD actually reads it

| object | briefed target | measured at `9595b54` |
|---|---|---|
| GPFIFO ring | guest's, verbatim | ★ `adopt=GUEST-RING`, `memory=0xcafe0006` |
| USERD | guest's, adopted | ★ `userd=GUEST-USERD`, same object, `+0x3000` |
| pushbuffer | guest's, never decoded | ⚠ **partial** — `CE-SUBMIT=0`, but the codec still ran once (§4) |
| doorbell | ours, token translation only | ★ `DOORBELL-XLATE … host_token=0x6 → WROTE` |
| error notifier | forwarded | ⊘ **not built by this rung** |

---

## 2. WHAT WAS REMOVED

**Fallback 1 — the second USERD object.** `alloc_channel_in`'s `Ours` arm no longer calls
`alloc_device_local` a second time. w284 measured the consequence of that second call across
three identical boots: ring leaf fb `0x40000` len `0x10000`, USERD fb `0x50000` — **the first
byte past the end** — so `adopted_guest_userd`'s containment test (`offset + 512 > len`)
declined. This boot reads `userd=h0xcafe000a/off0x3000/phys=fb:0x43000/0x200` against
`LEAF@0x120020000->0x40000/Vidmem/sz0x10000`: **inside**, by construction.

⊘ Deliberately **not** applied to the `RingSource::Guest{userd:None}` arm — there `ring_obj` is
the guest's object and putting our cursor inside it is `ShadowsGuestMemory` in a convenience's
clothes.

`UserdOwner::InRing` is a **third variant**, not a flavour of `Ours`, because `parts.userd` now
aliases `parts.ring`: teardown must free nothing, the CPU map must map nothing, and **all twelve
unwind paths** had to be re-derived from the owner rather than from `userd` — each would
otherwise have double-freed the aliased handle on the way out of an error.

**Fallback 2 — decode-and-re-emit on a passthrough channel.**
`ring_content_is_forwardable` was `engine == Ce`, full stop, with no flag. w283d's single CE
doorbell therefore rang the **adopted** channel (`host_token=0x6`) *and* decoded that same guest
ring and ran `ce_copy` on `host_token=0x7`. The green rows and the red row came from **two
different host channels in one boot**. It now takes `GuestChannelKind`, carried on
`DoorbellOutcome` off the same `chan` binding as `engine`.

**Added, not removed:** `USERD_OFFSET_MISALIGNED`. RM shifts the *resolved physical* USERD
address `>>9` (`kernel_channel_gv100.c:208`) and validates nothing, so a misaligned
`userdOffset` is **silently truncated** and presents as *"the cursor never moved"* — the exact
wall this rung removes. It is refused by name on all three arms, because on `HandedIn` the
offset is the guest's.

---

## 3. ★★★★★ THE WALL, AND THE OWNER'S GUARANTEE IS CONFIRMED — MEASURED, NOT INFERRED

The owner's 2026-08-13 ruling predicted: *"if the no fake FB assert does not hold I guarantee
you cup2 will never advance"*, and reasoned that deleting the CPU fallback converts a
working-but-wrong path into a dead one. **This boot is that prediction, reproduced.**

Our own address table, same boot, same doorbell:

```text
OPERAND-SOURCE-CE … operand(s)=2 (1 write, 1 read) [W@0x120010000+0x1000 R@0x120000000+0x1000]
OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 2 MISS
SEMA-TABLE:    1 page  asked, 0 resolved, 1 NOT-IN-GUEST-RAM [va=0x120022000:Vidmem@0x40000]
OPERAND-JOIN arm=off fb_join=shared ⇒ a CE operand page that lands in OUR EMULATED
  FRAMEBUFFER is LEFT THERE, SILENTLY — the default
```

⇒ The engine **fetched the entry** (cursor advanced), **executed the methods**, and the methods
**did nothing**. **No error. No Xid. No status.** That is the hang shape, and it is now
instrumented rather than hypothesised.

⚠ **But the two operand rows fail DIFFERENTLY, and the difference is load-bearing.** The
**semaphore** page *resolves* — to `Vidmem@0x40000`, i.e. the emulated framebuffer, which is
exactly the owner's fake-FB case and is promotable in principle. The **two copy operands** do
not resolve **at all**: `Miss`, no binding in the VAS. ⇒ Only the first is "fake FB reachable in
a passthrough VAS"; the second is *"the guest's allocation never entered our address table"*,
which no promotion mechanism addresses. §5.2 measures the consequence.

★★★ **And the source-level half of the owner's argument checks out too.**
`Representability::Fabricated` is defined in our own source as *"nothing host-side exists behind
it: it lives in the emulated framebuffer"* and routes to `CeExecutor::Ours`. **That CPU fallback
exists because operands land in fake FB** — it would never have been needed otherwise. It has
been doing the work whenever the GPU could not reach the bytes, which is why no green run ever
distinguished passthrough from the fallback catching it.

### ⊘ AND A STRUCTURAL LIMIT THE PROMOTION DESIGN MUST ABSORB

The owner's route table asks that an `NV_ADDR_FBMEM` object *"resolve to REAL host vidmem"*.
**It cannot, by a measured refusal already in this tree.** `ExportSource::HostDeviceMemory` —
*"the real device's own pages — host framebuffer, a channel's ring/USERD, a BAR0 register
window"* — is documented **`Always RmError::NotExportableAsMemory`**, and `8e245a8` measured it:
*"a Vidmem ring is NOT exportable as an fd"*.

⇒ `join_fb_leaf` therefore mints `ExportSource::Fabricated` — a **host memfd** — and wraps it in
an `NV01_MEMORY_SYSTEM_OS_DESCRIPTOR`. So an FBMEM-declared guest object resolves to **host
sysmem, mapped into the GPU VAS**, never to host vidmem. ⚠ This is *not* the catastrophic case
(that memory **is** hardware-reachable, which is why the cursor advanced); but it means **route
1 as stated is unbuildable, and route 2's mechanism is what actually serves both rows.** The
promotion the owner asked for is therefore "fake FB → **host-backed OS_DESCRIPTOR**", not
"fake FB → real vidmem", and that distinction should be settled before it is built.

---

## 3a. ★★★ THE BACKING CENSUS FOR THE PASSTHROUGH CHANNEL — measured, and it reshapes the ask

The requested census is *"on a `USER` channel, count objects whose backing is
`Representability::Fabricated`"*. Taken from the boot itself (`w287j_guest`, `proc=2 chan=0` —
the raw client's channel), **every VA the engine touches**:

| VA | role | what our table answers |
|---|---|---|
| `0x120020000` | GPFIFO ring | **`Vidmem`** — `LEAF@0x120020000->0x40000/Vidmem/sz0x10000` |
| `0x120022000` | release semaphore | **`Vidmem@0x40000`** — our emulated framebuffer |
| `0x120000000` | copy **source** | **`Miss`** — no binding in the VAS |
| `0x120010000` | copy **destination** | **`Miss`** — no binding in the VAS |

⇒ ★★★ **THE OWNER'S ASSERTION DOES NOT HOLD TODAY, AND THAT IS CONFIRMED:** fake framebuffer
**is** reachable in a guest userspace channel's VAS — the ring and the semaphore both resolve
into the emulated FB at `0x40000`.

⇒ ⊘⊘ **BUT THE WALL IS NOT THE ONE THE CENSUS WAS DESIGNED TO FIND, AND THIS CHANGES THE
PROMOTION DESIGN.** The two objects the copy actually reads and writes are **not fake-FB-backed —
they are `Miss`, unbound.** A promotion that swaps fake FB for host-backed memory **has nothing
to act on** for those two. The reasoning that *"the CPU fallback existed because operands land in
fake FB"* does not fit this channel: these operands were never resolvable at all, so the fallback
was not covering a fake-FB read here.

⚠ **AND THE HONEST LIMIT ON THE COUNT.** The literal string `Fabricated` appears **zero times**
in the whole boot log — but that is **not** evidence the count is zero, because **the log has
never been shown to print that word at all.** The same grep finds `Vidmem` twice and `Address`
twice, so the file and the pattern are live; the vocabulary `Vidmem`/`Miss` is what this log
speaks, and `Representability` is a decode-path type whose printing this rung's change
suppresses. ⇒ **Recorded as "the log does not carry that vocabulary", never as "the count is
zero"** — the `dlen=0` class, refused.

---

## 4. ⊘ WHAT THIS RUNG DID NOT CLOSE — stated, not absorbed

- **The copy does not execute.** `R33_RC=1`. Criterion 1's *cursor* bar is met; criterion 1 as a
  whole is not, and criteria 2/3 regressed to red **as predicted** — cutting the fallback
  removed the CPU copier that was making them green on a different channel.
- **Criterion 2 (deliberate fault, reported in the guest) was not run.** `--ce-client-fault`
  exists; this rung did not reach it.
- **The kind gate may be keyed on a proxy.** `SEMA-SOURCE-CE = 1` and `OPERAND-SOURCE-CE = 1` in
  a boot where the suppression should have made both `0`. `GuestChannelKind` is derived from the
  **proc anchor** (`SYSTEM_ANCHOR` vs not), which is *whose namespace allocated it* — **not** the
  brief's stated discriminator `internalFlags[1:0]` (`KERNEL(2)` vs `USER`/`ADMIN`). Either the
  raw client's proc is system-anchored, or the decode ran from a second site. ⚠ **Until that is
  resolved, "the pushbuffer is never decoded" is not established** — `CE-SUBMIT=0` says nothing
  was re-emitted, which is weaker.
- **The no-fake-FB assertion still has no passthrough-safe input.** `#255` is computed inside the
  CE decode path. `KAYFABE_OPERAND_JOIN=assert` is the existing three-arm control whose expected
  reading is *"#255 … FIRED"* — a positive observation with a built-in known-positive — and it
  should be the basis of the bind-time assertion rather than a new instrument.

---

## 5. THE OPTIONS, and what each costs

1. **Promotion at the ioctl boundary** (the owner's ask). Copy-and-swap fake FB → a host-backed
   `OS_DESCRIPTOR` inside the guest ioctl that first makes the object reachable to a passthrough
   channel. Race-free by construction: the vCPU is blocked, the channel does not yet exist.
   ⚠ Composes with *"RM zeroes 512 bytes of a handed-in USERD inside the alloc"* — same instant.
   ⊘ Needs §3's route-1 question settled first.
2. ⊘⊘ **`KAYFABE_OPERAND_JOIN=join` — MEASURED THIS RUNG, AND IT IS REFUTED.** Boot
   `w287j_guest`, same HEAD, the arm asserted out of the device's own line
   (`OPERAND-JOIN arm=join`), is **byte-for-byte the same result**: `GP_GET 1 GP_PUT 1`, methods
   did nothing, `R33_RC=1`, and the table still reads

   ```text
   OPERAND-TABLE: 2 page(s) asked, 0 resolved in guest RAM, 2 MISS
   SEMA-TABLE:    1 page  asked, 0 resolved, 1 NOT-IN-GUEST-RAM [va=0x120022000:Vidmem@0x40000]
   ```

   ★★★ **Why it cannot help, and this is the useful half:** the join is
   `resolve_leaf_of(ce, va)` — it **walks a VA to a framebuffer leaf**. The two operand VAs are
   `Miss`: there is **no binding at those VAs to walk**. ⇒ The join can only promote what the
   address table already resolves, and the operands never entered it.

   ⚠ Note the two failures are **different**, and reading them as one would misdirect the next
   rung: the **semaphore** resolves (`Vidmem@0x40000` — fake FB, promotable in principle), the
   **operands** do not resolve at all (`Miss` — nothing to promote). Promotion is necessary for
   the first and **insufficient for the second**.
3. **Resolve the kind gate** onto `internalFlags[1:0]` so the suppression is keyed on the
   owner's stated discriminator rather than on the anchor.

⊘ **None of these is "add tracking".** If promotion turns out to need a lock or a replay — i.e.
to happen outside a guest-blocking ioctl — that is the escalation the owner named, and it should
stop the lane rather than be engineered around.
