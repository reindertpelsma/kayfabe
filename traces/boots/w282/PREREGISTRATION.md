# w282 / PRE-REGISTRATION — LEG 7: the CE operand leaves, presented to the join

**STATUS: LIVE — 2026-08-13.** Committed **before** any boot of this rung.

## THE ONE VARIABLE

`KAYFABE_OPERAND_JOIN` — `join` on `w282_client` and `w282_cup2`, **unset** on `w282_clientoff`.
Every other flag is identical on all three arms and is **`w281b_clientsweep`'s exact arming**:
`FB_JOIN=shared GUEST_RING=ring GUEST_PUSHBUF=pin GUEST_SEMA=pin GR_ROUTE=passthrough
GUEST_OPERAND=pin PT_WITNESS_EXEC=on RING_VIDMEM=on PUSHBUF_VIDMEM=on PT_SWEEP=on`.

⊘ `PT_SWEEP` is **on on every arm** and is **not** this rung's variable: leg 7 selects its
candidates by reading the address table, and `w281` measured the operand VAs `2 MISS` without
the sweep — so a difference between the arms would be un-attributable between the two.

## WHY (the chain, all measured)

| boot | what happened |
|---|---|
| `w281_client` | a **real host copy engine executed the guest's own methods** and faulted `Xid 31 CE0 HUBCLIENT_CE1 FAULT_PTE ACCESS_TYPE_VIRT @ 0x1_20010000` — the dst the guest's pushbuffer declared |
| `w281b_clientsweep` | the sweep **bound** both operand VAs (`2 MISS → 0 MISS`) — to `Vidmem@0x10000` / `Vidmem@0x20000`, our **emulated** framebuffer ⇒ `Representability::Fabricated` ⇒ `CeExecutor::Ours` ⇒ `ce_copy` refuses by name |

⇒ Both reachable configurations are walls and both are the **same** missing thing. The join that
fixes it — `join_one_fb_leaf` — has existed since `w260` and is already driven off an **operand
census**; that caller hangs off `declare_gr_completion`, which `SharedDoorbell::ring` calls on the
two **GR** dispositions and on **no CE path at all**. **This rung is the caller.**

## PRE-REGISTERED HYPOTHESES

| # | prediction | what would falsify it |
|---|---|---|
| **H1** | `OPERAND-JOIN-TABLE` prints on `client`, **0 lines** on `clientoff` | a line on the control ⇒ the flag is not the variable |
| **H2** | the table names **2 CANDIDATE(S)** at `va=0x120000000` / `0x120010000`, `0 MISS` | any other VA ⇒ the decode changed under me |
| **H3** | **2 distinct leaves** join (`fb_phys=0x10000`, `0x20000`) — ⊘ *not* the ring's `0x40000` | 1 leaf ⇒ the operands share a leaf and my leaf arithmetic is wrong |
| **H4** ★★★★★ | **`#255` says `FIRED` on `clientoff`** naming both VAs, and **`QUIET` on `client`** | `QUIET` on the control ⇒ **the instrument did not run**, and every zero below is void |
| **H5** ★★★★★ | `CE-SUBMIT` still reads **`by=HostCe`** on `client` | `by=Ours` ⇒ a **regression wearing a green**, exactly `w281b`'s shape |
| **H6** | `HOST_DMESG_XID` = **0** on `client` while `by=HostCe` **and** `CE-SUBMIT ≥ 1` | ⊘ **all three together**, never the Xid alone |
| **H7** | the wall **moves** — a new fault VA (`0x1_2000_0000` the src, or `0x1_2002_2000` the semaphore) | staying at `0x1_2001_0000` ⇒ the join did not take |
| **H8** | the three criteria in the guest | not predicted; see the arms below |
| **H9** ⊘ | `cup2` holds at `CUP2_RC=124` | it is **not** a foregone conclusion — leg 7 keys on where the **operand** lands, not the pushbuffer |
| **H10** ⊘ | a VOID boot (md5 mismatch, `total≠53`, no `OPERAND-JOIN arm=` line) | the void guards |

## ⊘⊘ THE FALSIFIER IS ON **IDENTITY**, NOT MAGNITUDE

`w281b`'s pre-registered falsifier — *"`HOST_DMESG_XID` 1 → 0 while `CE-SUBMIT` stays 1"* — had
**both halves fire while the rung failed**, because `by=HostCe` → `by=Ours` substituted the thing
counted. **Third instance in three rungs.** The only reading that means progress here is:

> **`CE-SUBMIT` still says `by=HostCe`, AND `#255` went `FIRED`→`QUIET`, AND the Xid is gone or
> has MOVED to a different VA.** Any one of the three alone is not the result.

## THE ARMS, WIDENED — six, and the low ones are named

1. client passes all three criteria, and `cup2` moves.
2. client passes all three, `cup2` holds at 124.
3. the join binds (`#255` QUIET, `by=HostCe`) and the Xid **moves** to the src or the semaphore
   — a **stage**, and the cleanest possible one.
4. ★ the join binds but the executor **still** routes `Ours` — i.e. `JoinsGuestWindow` is not
   reaching `Representability::HostBacked`. That indicts the bind, not the join.
5. ★ the join **refuses** — most likely `Rm(NoMemory)` `0x51`, which is
   collision-or-exhaustion and ⊘ cannot be told apart from the status alone.
6. ★ `#255` stays `FIRED` on the armed arm ⇒ **pages were missed**, and we know before
   claiming anything.

⚠ Six of the last ten rungs had their least-weighted arm fire.

## WHAT THIS RUNG CANNOT PROVE, WHATEVER IT SAYS

- **It cannot say the operand join is the LAST blocker** — only whether it is *this* one.
- **The completion plane still has no oracle**; `sem=…` is our own read.
- **Cleanup is designed but NOT WIRED** (see the RESULT). A green here is a green with a
  known leak of joined leaves across the life of the VAS.
- One workload, one chip (GA106), one driver (`580.159.04`), one boot per arm.
