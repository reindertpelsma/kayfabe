# w245 boots — route B MEASURED, and it is UNREACHABLE **WITH THE EXECUTOR WITNESS DISARMED**

⊘⊘⊘ **SCOPED within the hour by `traces/boots/w246/` (§16.98) — and the scoping is mine to own.**
Both boots below ran with **`KAYFABE_PT_WITNESS_EXEC` unset**. That flag is what binds the ring's
VA: `w234a` (off) reports `rows=4 hit=NONE` — **byte-identical to these two** — and `w234b` (on)
reports `rows=13348 hit=0x1024000/Vidmem`, `RING-VA-UNBOUND` **0**, `PushbufferAperture` **9**.
⇒ **"route B is unreachable" is true of a CONFIGURATION; I wrote it about the CODE.** The wall did
not move — §16.86 measured it with the witness armed and I did not arm it. Read `w246`'s README.

⚠ **`CE-SUBMIT` is 0 on both arms and nothing executed.** ⊘ No line here may be read as the
first forwarded work. **The named wall is the deliverable.**

Source revision **`acbb9a3`**, stamped inside both artifacts. ⊘ HEAD is `30f7900` and that is
**not** a stale binary: `git rev-parse acbb9a3:crates` / `:tests` / `:Cargo.lock` are
byte-identical to `30f7900`'s — the later commit is docs and evidence only, so `acbb9a3` is the
complete attribution.

## Two arms, one variable

| | `w245off_30f7900_rbo` | `w245on_30f7900_rbn` |
|---|---|---|
| `KAYFABE_RING_VIDMEM` | `0` | `1` |
| `KAYFABE_CE_EXECUTOR` | `host` | `host` |
| `RING-PROJ` (fall-through entered) | 8 | 8 |
| **`RING-VA-UNBOUND`** | **8** | **8** |
| `NOTHING FORWARDED` | 8 | 8 |
| `PushbufferAperture` | 0 | 0 |
| `RingFbNeverWritten` (forbidden #2) | **0** | **0** |
| **`CE-SUBMIT`** | **0** | **0** |
| `no-blocking-under-lock` | 0 | 0 |
| doorbells | 191 / 183 / 8 | 191 / 183 / 8 |
| `SMI_RC` / `CUP2_RC` | 0 / 124 | 0 / 124 |

★★ **Whole-log normalised diff — 808 lines each, ONE distinct line:**

```
$ norm() { sed -E 's/^[0-9-]+T[^ ]+ //; s/0x[0-9a-f]+/HEX/g; s/[0-9]+/N/g' "$1"; }
$ diff <(norm …off_qemu.log | sort -u) <(norm …on_qemu.log | sort -u)
35c35
< kayfabe: RING-VIDMEM KAYFABE_RING_VIDMEM=N ⇒ route B OFF (default)
> kayfabe: RING-VIDMEM KAYFABE_RING_VIDMEM=N ⇒ route B ON
```

⇒ The control arm reproduces the wall **exactly** (and matches `w244a`, an independent boot at
the same revision), and the flag changes nothing else.

## Why — the wall is the ADDRESS TABLE, not the aperture

All 8 candidate `proc 2` doorbells:

```
GUEST-RAM PIN … ring=0x200224000 → UNRESOLVED Address(Miss { pdb: Pdb(2101248), … })
                                   (the address table does not bind this VA; ⊘ MISS = FAULT)
FWD-RING proc=2 chan=12 … RING-VA-UNBOUND va=0x200224000 → NOTHING FORWARDED
```

`plan_gpfifo_ring` returns `RingVaUnbound` at its `binding_at` miss (`kayfabe-fwd/src/lib.rs:4258`)
— **before** `VidmemRoute` is computed (`:4277`) and long before `fetch_ring_bytes`, where
forbidden #2's residency gate lives (`:4316`). ⇒ route B is **wired, functional and unreachable**.

## ⊘ The forbidden-#2 residency gate, answered plainly

**It did not fire on hardware, and it could not have.** Not *"still not"* — **unreachable**,
because it is downstream of the exit all 8 candidates take. It remains proven offline (4 arms,
negative control watched red). ⇒ an outstanding debt whose **cause is now named**.

## ⊘ Why `ce_executor=host` and not the default

`[measured at acbb9a3]` `RING-PROJ` is **8** on `host` and **0** on `local` — with `local`,
`try_ce_submission` claims every routed doorbell terminally and the fall-through is dead code.
Running this on the default would have produced "identical arms" for a **second, unrelated**
reason: the right answer for the wrong cause.

## The next question, with its numbers

`pdb=0x201000`, all 8: `VAS-BIND-CENSUS … vas=PRESENT rows=4 hit=NONE`, and `PT-DECODE`
`bound=6275 unwitnessed=6275 learned=39 published=39/0` on the **first** doorbell, `bound=0` on
the other seven. 6275 bindings decoded; the ring's VA is in none of the VAS's 4 rows.

Full record: `docs/design/execution_plane_increments.md` §16.97.
