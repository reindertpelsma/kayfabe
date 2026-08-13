# w267 PRE-REGISTRATION — **READ THE 4 KiB PAGE.** Committed before the code exists

> ### STATUS — 2026-08-12 / **LIVE — PRE-REGISTRATION, written before one line of the instrument.**
> Branch `w267-read-the-page`, off `leg-5-completion-pin` = `7010ea2`.
> Predecessor: `traces/boots/w266/RESULT.md` (rev `f09aba2`, 2 arms, real GA106).
> Bench `vh` = `NVIDIA GeForce RTX 3060` (GA106), host driver `580.159.04`.

---

## 0. ⊘⊘ THREE THINGS IN THE BRIEF ARE ALREADY REFUTED, AND ONE OF THEM IS ITS FOLLOW-THROUGH

The brief is right that the rung is *"dump the page"*. It is wrong about **who** the missing
write belongs to, and therefore about what to do if the dump is favourable. All three
refutations come out of `w266`'s own logs, before any new boot.

### 0.1 ★★★★★ THE OBSERVER ALREADY WATCHES THE RIGHT OFFSET — all eight of them, derived from the guest's own declaration

The brief's follow-through reads: *"if a payload landed at an offset the observer does not
watch, the fix is to watch the **right** offset — and the right offset is whatever the guest's
own `SET_REPORT_SEMAPHORE` names … derive it from the guest's declaration, do not hardcode
`0xff0`."*

⊘ **That is already what the code does, and it has been doing it since `w226`.** The observer's
addresses are not hardcoded: `decode_report_semaphore` reads
`(A[7:0] << 32) | B[31:0]` out of the guest's own four-word run, class-gated on the
`SET_OBJECT` that bound the subchannel, and `WatchKey::va` *is* that decoded VA.
`[measured, w266_on, `run_w266_on_qemu.log`]` **eight** `COMPLETION-DECLARE` lines, one per GR
channel, at **eight distinct VAs**:

```
proc=2 chan=0 → va=0x20440fff0   site=GuestRam { gpa: 563605488 } = 0x2197eff0
proc=2 chan=1 → va=0x20440ffe0   … chan=2 ffd0, 3 ffc0, 4 ffb0, 5 ffa0, 6 ff90, 7 ff80
```

and **eight** `COMPLETION-WATCH` verdicts against those same eight VAs. The brief's arithmetic
(*"the observer watches page base + 0xff0"*) is true of **one** of the eight and reads as though
it were the whole watch. ⇒ **There is no "right offset" left to derive for the GR plane.** A rung
spent widening the GR watch would be a rung spent on a capability that is already complete —
the tenth consecutive instance of the class, and the brief itself names the class.

### 0.2 ★★★★★ THE ENGINE THAT FAULTED IS **NOT** THE ENGINE THE OBSERVER WATCHES — different channels, different class, different completion

This is the load-bearing correction and it changes what a favourable dump would *mean*.

`[measured, w266_off, `run_w266_off_hostdmesg.log`, all 8 lines]` the eight `Xid 31` name
**channel `0x01000011…0x01000014` and `0x02000015…0x02000018`**, `ENGINE CE2`/`CE3`,
`HUBCLIENT_CE0`/`CE1`. `[measured, w266_on, `run_w266_on_qemu.log` lines 622-716]` those are the
device's **`engine=Ce`** channels — procs `2`, chans `8…15`, tokens `0x0001000f…0x00020016` —
and every one of them carries a **32-byte pushbuffer**:

```
pbm[8w of 32B]: [0]sub4/m0x0/Incrementing/n1=0xc7b5
                [1]sub4/m0x240/Incrementing/n3=0x2
                [2]sub4/m0x300/Incrementing/n1=0x14
```

Decoded against `ogkm-580.159.04: src/common/sdk/nvidia/inc/class/clc7b5.h`:

| word | method | meaning |
|---|---|---|
| `sub4 m0x0 = 0xc7b5` | `SET_OBJECT` | `AMPERE_DMA_COPY_B` — **the copy engine**, not `0xc7c0` |
| `sub4 m0x240 n=3` | `SET_SEMAPHORE_A` / `_B` / `_PAYLOAD` (`:47-52`) | ⚠ **`_A_UPPER` is `16:0` here, not `7:0`** as on the compute class |
| `sub4 m0x300 = 0x14` | `LAUNCH_DMA` (`:84-105`) | `DATA_TRANSFER_TYPE = NONE` (`1:0` = 0), `FLUSH_ENABLE = TRUE` (`2:2` = 1), `SEMAPHORE_TYPE = RELEASE_FOUR_WORD_SEMAPHORE` (`4:3` = 2), `INTERRUPT_TYPE = NONE` (`6:5` = 0) |

⇒ ★★★ **Each CE channel submits a pure four-word semaphore release that moves no data.** That
is the write that faulted, and it belongs to the copy engine's own `SET_SEMAPHORE`, on a
channel the observer has never had a watch on — the observer's eight watches are all
`engine=GrCompute`, class `0xc7c0`, `SET_REPORT_SEMAPHORE`.

⇒ **World (a) and world (b) are both statements about the CE's semaphore, and NEITHER of them
is a statement about the guest's `cuCtxCreate` completion.** Even a maximally favourable dump —
a payload sitting at the CE's own offset — leaves the eight GR report semaphores untouched,
because **the GR channels' work has not run**. `CE-SUBMIT` is `0` and `DOORBELL-REFUSED` is
`16`, on both `w266` arms.

### 0.3 ⊘ THE THIRD ARGUMENT IS ALREADY ON DISK AND MERELY UNPRINTED — this is an ARMING, not a build

The brief asks *"whose target that offset is — CE `SET_SEMAPHORE` vs GR `SET_REPORT_SEMAPHORE`
vs neither"*. The CE's answer is **in the guest's own pushbuffer, already read, already
decoded, and truncated at the print**: `push_headers` (`shim.rs:7041-7045`) emits
`words[i + 1]` and stops, so `n=3` renders as `=0x2` — the *upper* half of the semaphore
address — and `_B` (the low 32 bits) and `_PAYLOAD` are dropped on the floor. The comment above
that line even says what it is: *"a semaphore's address half"*.

⇒ One of this rung's two changes is **deleting a truncation**, not writing a decoder.

---

## 1. WHAT IS BUILT — two instruments, both consumers of capabilities that already exist

⊘ **No new capability.** `Vmm::gpa_read` already takes `&mut [u8]` of any length and the
observer thread already holds a `QemuVmm`; `WatchList::declared_sites` already hands out every
declared `(WatchKey, Site)`. What does not exist is a **consumer** that reads more than four
bytes. `[verified]` the only production `gpa_read` call site in the tree is the observer's
`&mut [u8; 4]` closure at `shim.rs:3419-3421`; every other caller is a test.

**I1 — `SEMA-PAGE`, the 4 KiB dump.** On the observer thread, after each sweep: for every
distinct 4 KiB page containing a declared `Site::GuestRam` address, read the whole page and
print **every non-zero 4-byte slot with its offset**, an **exact** non-zero total, and an
explicit row for **each declared VA's offset whether or not it is zero**. Each dump carries a
sequence number and `t=+Nms` since the observer started, so *when* is answerable; dumps are
emitted on first sight, **on every content change**, on a 5 s heartbeat, beside every verdict,
and once at teardown.

**I2 — un-truncate `push_headers`.** Print every argument word of each shown method, as
itself, uninterpreted. The CE's `SET_SEMAPHORE_A/B/PAYLOAD` becomes readable.

**Arms.** Two, one variable, exactly `w266`'s: `KAYFABE_GUEST_SEMA` `off` → `pin`. Both
instruments are on **both** arms — they are instruments, not the variable. ★ The `off` arm is
the negative control the brief did not ask for and it is the sharpest row here: on `off` the
engine **faults instead of writing**, so any content that appears only on `on` is
engine-written by elimination.

⚠ Both are print-only and neither writes. `completion_watch.rs` is untouched: the module's
guarantee is that `WatchList` is handed a *reader*, and this rung does not widen it.

---

## 2. THE PREDICTIONS — named before the dump exists

★ **Calibration inherited from the predecessor, and acted on rather than restated**: two
consecutive rungs had their *least*-weighted branch fire, both favourably. So every arm below
that I would previously have left unnamed is named and given mass, and the residual is
explicit. ⊘ I have not simply moved probability toward the good outcome; I have widened the
tails in both directions, because an unnamed outcome is uninterpretable however it lands.

### 2.1 The dump's readings — the complete space, named before seeing it

| # | reading | what it would mean | p |
|---|---|---|---|
| **W1** | `on` page has non-zero at an offset **below `0xf80`**, `off` page does not | **(a) — the CE wrote.** The pin closed the CE's write and it landed at the CE's own slot. | **0.45** |
| **W2** | both arms all-zero across the whole 4 KiB, every sample | **(b) — nothing wrote, and nothing ever had.** The `Xid` going to zero was the *mapping* being installed, and the engine still never completes. | **0.25** |
| **W3** | both arms carry the **same** non-zero content, unchanging | the page has **guest-CPU-written** content (a seed, a header) and no engine write. Neither (a) nor (b) as stated — *"non-zero"* would not have been evidence at all. | **0.15** |
| **W4** | `on` page changes **over time** (two dumps differ) | the strongest form of (a): a *sequence* of engine writes, and the timestamps give the rate. | **0.08** |
| **W5** | non-zero appears at an offset **in `0xf80…0xfff`** — i.e. inside the GR slots — with `last_seen` still `0` | ⚠ the two projections disagree: the observer reads through `gpa_read`, the dump reads through the same call, so this would indict the **payload comparison**, not the plane. Would need the raw slot vs `payload=1`. | **0.04** |
| **W6** | `gpa_read` **refuses** the page on either arm | a statement about the instrument only. ⊘ Named so a refusal cannot be read as emptiness — this is the `dlen=0` class and it gets its own row. | **0.03** |

⊘ **An all-zero page (W2) is a RESULT, not a failed measurement**, and it is the one that
changes the diagnosis most: it retires the *"landed at a slot nobody watches"* story entirely
and says the completion plane is not merely mis-observed but **absent**.

### 2.2 The CE semaphore address, from I2 — a falsifiable arithmetic prediction

`SET_SEMAPHORE_A` arg `= 0x2` is measured. The address is
`((A & 0x1ffff) << 32) | B`, so the VA is `0x2_xxxxxxxx` and the fault says the page is
`0x2_0440f000`.

| # | prediction | p |
|---|---|---|
| **C1** | `B` decodes into the page `0x0440f000`, at an offset **below `0xf80`** | **0.70** |
| **C2** | `B` decodes into the page `0x0440f000` at an offset **inside `0xf80…0xfff`** — i.e. the CE and a GR channel share a slot | **0.08** |
| **C3** | `B` decodes **outside** that page, and the fault address is a PDE-level truncation of a different VA in the same 2 MiB | **0.20** |
| **C4** | the three arguments do not decode as `A/B/PAYLOAD` at all (the run is not what `clc7b5.h` says) | **0.02** |

★ If **C1 and W1 agree on the same offset**, that is the whole rung answered in one line: the
engine wrote where its own operand said, and the observer was watching a different completion.
⊘ If **C1 holds and W1 fails**, we are in (b) *with the target known* — the engine had a
mapping and a target and still did not write — which is a much stronger and more useful
negative than `w266` could produce.

### 2.3 The carried rows — predicted at **zero movement**, for the seventh consecutive rung

| observable | prediction | why |
|---|---|---|
| **`CUP2_RC`** | **`124` on both arms**, `p = 0.93` | ★ **And the size, as asked: I predict movement of ZERO — not "small".** ⊘ The only path to any other value runs through a GR report semaphore reaching payload `1`, and this rung submits nothing, routes nothing, and refuses the same 16 doorbells. The residual `0.07` is *not* "it might just work": it is the mass on `cup2` changing for a reason unrelated to this rung (a boot-order flake, a different timeout), which would make the number **uninterpretable**, not good. |
| `CE-SUBMIT` / `RETIRED` | `0` / `0`, both arms | nothing here submits. The seventh boot in a row where this is pre-registered at zero. |
| `COMPLETION-WATCH → OBSERVED` | `0`, both arms, `p = 0.95` | the GR channels' work has not run — §0.2 |
| `COMPLETION-WATCH → NOT-OBSERVED` | `8`, both arms | |
| host `Xid` COUNT / ENGINE / CLIENT / DISTINCT-ADDRS / ACCESS-TYPE | `off`: `8` / `CE3`+`CE2` / `HUBCLIENT_CE1`+`CE0` / `1` / `VIRT_WRITE`. `on`: `0` / — / — / `0` / — | ★★★ graded as **identity**, never as a count: `w265` moved five facts under a count that read `8` on both arms |
| `PB-PIN token=` | `16` both | leg-4 guard |
| `PT-DECODE` first pass | `bound=19615 unwitnessed=19874` both | ★★ **cross-revision guard** — this binary is not `w266`'s, and if these move the comparison is void |
| `refusals=` | `255` both | the carried, unpaid debt — unchanged, this rung touches nothing |
| `SEMA-PIN` lines / `sema run … placed_as_asked=true` | `0` / `0` on `off`; `16` / `8` on `on` | leg-5 guard, `w266`'s corrected values |

### 2.4 ⚠ Predictions about the instruments themselves

- **I1 prints on both arms** — a `SEMA-PAGE` count of `0` on either arm means the observer never
  ran or nothing was declared, and is a statement about the instrument. Predicted `> 0` on both,
  `p = 0.9`; the `0.1` is *"the observer thread failed to start"*, which prints its own line.
- **I2's line grows, and `[N]sub…=0x…` becomes `[N]sub…=[0x…,0x…,0x…]`.** ⚠ No grader in the
  tree greps it (`grep -rn pbm scripts/` = 0 hits), so this re-scopes nothing.
- ⚠⚠ **The `SEMA-PAGE` label is chosen to collide with NOTHING**, and this was checked against
  `w266_grade.sh` line by line rather than assumed. It must not contain `SEMA-PIN token=`,
  `[^-]TABLE:`, `PINNED`, `placed_as_asked`, `CAPPED`, `MISS`, `RETIRED`, `GP_PUT`,
  `NOT IN GUEST RAM` or `refusals=` — every one of those is a live regex in `w266_grade.sh`, and
  `w266`'s own most expensive instrument lesson was **a new producer silently re-scoping three
  consumers that were implicitly scoped by being the only one**. The truncation notice is spelled
  `LISTING-BOUND`, deliberately not `CAPPED`, for exactly that reason.

---

## 3. ⊘ WHAT THIS RUN WILL NOT BE ABLE TO PROVE — stated now, so it cannot be quietly dropped later

1. ⊘ **That the guest's CPU sees what the dump sees.** `gpa_read` reads through the **VMM's**
   view of the memfd, not through the guest's own mapping of it. Same memfd, so it is *likely*;
   it is not measured, and `w266` §4 already carries this limit unpaid.
2. ⊘ **That an absence of change between two dumps is an absence of writes.** A write of the
   same bytes is invisible to a content signature. The dump reports *state*, never *events*.
3. ⊘ **Anything about the GR completion plane.** §0.2. Whatever the CE did, the eight
   `SET_REPORT_SEMAPHORE` targets are written by GR work that has not run.
4. ⊘ **Which VAS the host channel is bound to.** Still `[NOT MEASURED]`, carried from `w266`.
5. ⊘ **The 255 `StraddlesLiveBinding` refusals** and **`by-executor = 39`** — untouched.
6. ⊘ **Ordering.** The dump samples at 250 ms; a write and an immediate overwrite between two
   samples is unobservable by construction.
