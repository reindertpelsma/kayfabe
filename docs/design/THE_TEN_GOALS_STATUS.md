# The ten goals — measured status

**STATUS: LIVE, 2026-09-13 (w619).** Every row cites a measurement from a boot on a **matching**
GA106 (RTX 3060, driver 580.159.04) that graded the raw client `(P)`. ⊘ A row with no measurement
says so; *"not started"* and *"believed fine"* are different states and are not merged.

## The table

| # | goal | state | the measurement |
|---|---|---|---|
| 1 | Blackwell boots | **not started** | — |
| 2 | zero read traps; write traps only in BAR0, not PRAMIN, not BAR1/2 | **one page short** | `pages_touched=1`, all the counter |
| 3 | no blocking calls or held locks on the vCPU | **met, with one caveat** | `inline_exceptions=0` every boot; `VCPU-BLOCKING none`; rank-0 `worst_wait=0us slow_waits=0` |
| 4 | the DoorbellTable wired | **not started** | `dbtable.rs` has ZERO callers; it replaces a 630-line path reaching 14 subsystems |
| 5 | code rot cleaned or marked | **advanced** | three censuses adjudicated (w605), 16 unrecoverable boot tags grandfathered, full workspace gate green |
| 6 | every write trap sub-millisecond | **one trap over** | `slow_traps(>1ms)=1`, `worst_trap≈17ms at bar0+0x110c00` |
| 7 | raw client passing the mean test | **MET** | `W392D_GUEST_OUTCOME=(P)`, `THREADS 8 of 8 verified`, `MEAN_FALSIFIER=PASS`, on every boot this session |
| 8 | TWO raw clients in parallel (needs epoll in workers) | **not started** | — |
| 9 | the LLM working, then at parity | **not started this session** | last known: `the_llm_fails_on_all_three_doorbell_arms` |
| 10 | more CUDA apps, then all of it on Blackwell | **not started** | — |

## ★★★★★ GOAL 2, RE-GRADED AT HEAD (w632) — one page touched, and it is the counter

`[measured w631a, rev 9728dbc9, a boot that graded `(P)`]` — **nine commits after the BAR work
was first measured**, because a result that old is a claim about a tree that no longer exists:

    BAR1 (translated)   0 reads / 0 writes
    BAR2 (translated)   0 reads / 0 writes
    PRAMIN-ONLY         0 reads / 0 writes
    BAR0-READ-HOTSPOTS  pages_touched=1  reads_from_live_pages=129
                        reads_from_BACKED_pages=0   top[+0xbb0000=129]
    W392D_GUEST_OUTCOME=(P)

⇒ **`pages_touched=1`.** One page on the entire device produces a read trap, it is the
free-running counter, and `reads_from_BACKED_pages=0` says nothing leaked out of a page the cut
is supposed to serve.

## Goal 2, in detail — the one that moved

| surface | reads | writes |
|---|---|---|
| BAR0 excluding the counter page | **0** | traps — the control plane, which is allowed |
| the free-running counter (`+0xbb0000`) | **~132** ⊘ the only read traps left | — |
| PRAMIN | **0** | **0** |
| BAR1 | ~0–55 | ~2 300–3 500 |
| BAR2 | ~2 | ~1 450–1 600 |

★ **BAR0's read surface went 184 585 → ~134** and every remaining read is the counter page.
★ **PRAMIN is at zero in both directions, proven against its own control** — `KAYFABE_PRAMIN_SLOT=0`
measures 22 / 67 956, the default measures 0 / 0, same binary, both arms `(P)`.

### What goal 2 still owes, and what is known about each

**The counter page.** It cannot be shadowed — it changes continuously — so the only answer is to
map the HOST's own usermode page read-only. `the_counter_page_and_the_device_view.md` carries the
design, and its containment is **measured**: an `O_RDONLY` device node refuses a writable `mmap`
with `EACCES` and refuses `mprotect` back to writable with the same errno, both unprivileged.
⊘ The wire verb that ships the descriptor to the VMM is **not built**.

**BAR1/BAR2.** `[measured w608]` 184 distinct pages cost 3 952 trapped accesses — 21 per page —
because fills are on demand and the guest re-touches a page before its slot lands. 89.2 % of that
working set (100 % of BAR1's) is needed only after the first channel birth, so the owner's
map-at-create ruling is aimed at the right traffic.
⊘⊘ **The first implementation FAILED** (w618): enumerating BAR1's leaves mapped 253 pages the
guest never touches, made `ALREADY-COVERED-EARLY` 4.5× worse, and did not reduce BAR1's traps at
all. Default off behind `KAYFABE_PREMAP_BAR1=1`. **Why premapping did not reduce the traps is
unexplained** and is the open question.

## Goal 6's one trap, and why it is not excused

`worst_trap≈17 ms at bar0+0x110c00` (the GSP RPC submit), once per boot. The owner's rule excuses
unscheduled time only when it is a vCPU steal. `[measured w593, two boots]`:

    wall 18 331 us   thread_cpu 18 098 us   off_cpu   233 us
    wall 16 956 us   thread_cpu 16 771 us   off_cpu   185 us

⇒ **98.9 % of it is the thread RUNNING.** It is our own work, the excuse does not apply, and the
site is one-time by shape (`slow_traps=1` while 359 doorbells and 1 181 invalidates pass cleanly).

## Goal 3's caveat

`inline_exceptions=0` and every lock census is clean, so nothing BLOCKS on the vCPU. ⊘ But goal 6's
17 ms is CPU burned inside an MMIO exit, which is the vCPU stopped for 17 ms without blocking on
anything. The two goals disagree about whether that is a violation; goal 6 is the one it fails.
