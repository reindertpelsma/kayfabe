# w267 — **THE COPY ENGINE WROTE THE PAGE.** World (a), measured, with a per-channel control inside one boot

> ### STATUS — 2026-08-12 / **LIVE — MEASURED.** Two arms from **one** binary, source revision
> **`b129770`**, stamp-gated against the binary before booting
> (`STAMP: [kayfabe-rev:b12977083…] WANT: [kayfabe-rev:b12977083…]` → `PASS`), content-checked on
> **16** strings including this rung's own five, and four arming assertions per arm read out of
> that arm's own log (`GUEST-SEMA`, `EXEC-WITNESS`, `GUEST-PUSHBUF` — all **PASS** on both).
> Bench `vh`, `NVIDIA GeForce RTX 3060` (GA106), host driver **580.159.04**.
> Graded against `docs/design/w267_read_the_page_prereg.md`, committed at **`efbcaba`**,
> **before either instrument existed** (instruments landed at `2d9ed47`, harness at `b129770`). `BUILD_RC=0`, both `BOOT … RC=0`,
> `EXIT rc=0`, `ENOSPC/LLVM = 0` from the same invocations. Branch `w267-read-the-page`.

---

## 0. ★★★★★ THE HEADLINE — the write LANDED, and the page names which channel wrote it

**`w266` left two worlds open. The dump closes them, and it does not need the two arms to do it:
the `on` arm alone contains a complete per-channel control.**

```
kayfabe: SEMA-PAGE seq=9 why=tick t=+71169ms gpa=0x54c6000 len=4096 declares=8
         nonzero=12/1024 sig=0xbc09102bb7f6bcde first=0 changed=1
  SEMA-PAGE-NZ +0xf40=0x00000001 +0xf48=0x0de6d440 +0xf4c=0x18cb063c
               +0xf50=0x00000001 +0xf58=0x0ca34340 +0xf5c=0x18cb063c
  SEMA-PAGE-NZ +0xf60=0x00000001 +0xf68=0x0b632fa0 +0xf6c=0x18cb063c
               +0xf70=0x00000001 +0xf78=0x0a409300 +0xf7c=0x18cb063c
  SEMA-PAGE-SLOT +0xf80 … +0xff0   report16=[0,0,0,0]  ×8   ← every GR slot, still zero
```

**Four complete `RELEASE_FOUR_WORD_SEMAPHORE` reports** — `[payload=1, 0, timestamp_lo,
timestamp_hi]` — at `+0xf40`, `+0xf50`, `+0xf60`, `+0xf70`.
★★★ **The timestamps are the proof of authorship.** `0x18cb063c_0de6d440` and its three
siblings are a GPU nanosecond counter, monotonically ordered and distinct per channel. Nothing
in this VMM has that clock or writes that field. ⊘ This is not *"a value we expected appeared"*
— that could be argued into a coincidence — it is **a value we could not have fabricated**.

⇒ **WORLD (a) IS MEASURED. WORLD (b) IS REFUTED.** `w266`'s `Xid → 0` was a write **landing**,
at a slot nobody was watching, exactly as the evidence leaned — and leaning has been replaced by
reading.

### 0.1 ★★★★★ THE ROW THAT MAKES IT CAUSAL — eight channels, one boot, and the split is EXACTLY the pin

The `on` arm hit an ordering race that `w266` did not (§3.1), and it turned the boot into a
controlled experiment finer than the two arms are. **Four of the eight CE doorbells arrived
before any completion had been declared, so four pins never happened.** Every downstream fact
partitions on that line and on nothing else:

| token | CE `SET_SEMAPHORE` target | `SEMA-PIN` verdict | slot in the page | host `Xid` |
|---|---|---|---|---|
| `0x0001000f` | `0x2_0440ff70` | `SOURCE 8 declared` → **PINNED** | `+0xf70` = **`1` + ts** | — |
| `0x00010010` | `0x2_0440ff60` | `SOURCE 8` → ALREADY PINNED | `+0xf60` = **`1` + ts** | — |
| `0x00010011` | `0x2_0440ff50` | `SOURCE 8` → ALREADY PINNED | `+0xf50` = **`1` + ts** | — |
| `0x00010012` | `0x2_0440ff40` | `SOURCE 8` → ALREADY PINNED | `+0xf40` = **`1` + ts** | — |
| `0x00020013` | `0x2_0440ff30` | `SOURCE 0` → **NO PAGE TO PIN** | `+0xf30` = **`0`** | ✔ ch `0x02000015` |
| `0x00020014` | `0x2_0440ff20` | `SOURCE 0` → **NO PAGE TO PIN** | `+0xf20` = **`0`** | ✔ ch `0x02000016` |
| `0x00020015` | `0x2_0440ff10` | `SOURCE 0` → **NO PAGE TO PIN** | `+0xf10` = **`0`** | ✔ ch `0x02000017` |
| `0x00020016` | `0x2_0440ff00` | `SOURCE 0` → **NO PAGE TO PIN** | `+0xf00` = **`0`** | ✔ ch `0x02000018` |

★ **Eight for eight, with no exceptions and no interpretation.** Pinned ⇒ payload written,
timestamped, no fault. Unpinned ⇒ slot zero, `Xid 31 … ACCESS_TYPE_VIRT_WRITE`. Same boot, same
page, same engine class, same 250 ms observer.
⊘ **And the `off` arm is the outer control**: `KAYFABE_GUEST_SEMA` unset, **zero** pins, **8**
`Xid`, and the page **`nonzero=0/1024` on all twenty dumps over 174 seconds**.

### 0.2 ⊘⊘ AND THE GR HALF IS UNTOUCHED — which is §0.2 of the pre-registration, measured

`COMPLETION-WATCH … NOT-OBSERVED`, **8**, `last_seen=0x00000000`, on **both** arms.
`CUP2_RC = 124` on both. `CE-SUBMIT` / `RETIRED` = `0` / `0`.

★★★ **The page holds SIXTEEN 16-byte semaphore slots and they belong to two different engines:**

```
+0xf00 … +0xf70   the eight CE channels' NVC7B5 SET_SEMAPHORE targets   ← WRITTEN (4 of 8)
+0xf80 … +0xff0   the eight GR channels' NVC7C0 SET_REPORT_SEMAPHORE    ← ALL ZERO, always
```

⇒ The engine that stopped faulting is **not** the engine `cuCtxCreate` is waiting on. The rung
this closes was never on the path to `CUP2_RC`, and the pre-registration said so before the boot.

---

## 1. ⊘⊘ WHAT CONTRADICTS THE BRIEF — three things, and one of them is its follow-through

Stated first, as asked.

### 1.1 ★★★★★ "Derive the right offset from the guest's `SET_REPORT_SEMAPHORE`" — the code has done that since `w226`

The brief's IF-(a) branch reads *"the fix is to watch the **right** offset … derive it from the
guest's own declaration, do not hardcode `0xff0`."* ⊘ Nothing is hardcoded and nothing is
missing. `decode_report_semaphore` reads `(A[7:0] << 32) | B[31:0]` out of the guest's own
four-word run, class-gated on the `SET_OBJECT` that bound the subchannel, and `WatchKey::va` **is**
that decode. `[measured, this boot, both arms]` **eight** declares at **eight distinct VAs**
(`0x20440ff80 … 0x20440fff0`) and **eight** verdicts against those same eight. The brief's
arithmetic — *"the observer watches page base + 0xff0"* — is true of **one of the eight** and
reads as if it were the whole watch.
⇒ **There was no widening to do**, and a rung spent on it would have been the eleventh
consecutive lane to build something already built.

### 1.2 ★★★★★ The engine that faulted is not the engine the observer watches

The eight `Xid` are the **copy engine's own** `NVC7B5_SET_SEMAPHORE`, on `engine=Ce` channels the
observer has never had a watch on. `[measured]` each CE channel's whole pushbuffer is 32 bytes:

```
[0]sub4/m0x0  /n1=[0xc7b5]                    SET_OBJECT  = AMPERE_DMA_COPY_B
[1]sub4/m0x240/n3=[0x2,0x440ff70,0x1]         SET_SEMAPHORE_A / _B / _PAYLOAD
[2]sub4/m0x300/n1=[0x14]                      LAUNCH_DMA
```

`0x14` decodes (`ogkm-580.159.04: clc7b5.h:84-105`) as `DATA_TRANSFER_TYPE = NONE`,
`FLUSH_ENABLE = TRUE`, `SEMAPHORE_TYPE = RELEASE_FOUR_WORD_SEMAPHORE`, `INTERRUPT_TYPE = NONE`
— **a pure semaphore release that moves no data**, which is exactly the 16-byte report the dump
found. ⚠ `SET_SEMAPHORE_A_UPPER` is `16:0` on this class, **not** the `7:0` of the compute
class's `SET_REPORT_SEMAPHORE_A`.
⇒ Worlds (a) and (b) were **both** statements about the CE's write and **neither** was a
statement about `cuCtxCreate`'s completion. §0.2.

### 1.3 ⊘ The address the brief wanted derived was already read, and thrown away at the print

`push_headers` emitted `words[i + 1]` and stopped, so a three-argument run rendered as `=0x2` —
the `_A` half alone, with the low 32 bits (**the entire offset within the faulting page**) and
the payload dropped. The comment above that line called it *"a semaphore's address half"*: it
**named the defect and kept it**. ⇒ One of this rung's two changes is **deleting a truncation**.
★ For a multi-word operand a one-argument dump is not a smaller dump, it is a **wrong** one —
the half it keeps carries the least information.

---

## 2. THE PRE-REGISTERED SCORECARD

Transcribed from `w267_grade.txt`, which is `scripts/bench/w267_grade.sh`'s output, not a reading.

| # | observable | predicted | **off** | **on** | |
|---|---|---|---|---|---|
| **W1** | non-zero below `0xf80` on `on` only | `p = .45` | `nonzero=0/1024` ×20 | **`12/1024`** at `+0xf40…+0xf7c` | ★★★★★ **FIRED** |
| **W2** | both arms all-zero | `p = .25` | — | — | ⊘ refuted on `on` |
| **W3** | same non-zero on both | `p = .15` | — | — | ⊘ refuted — `off` is 100 % zero |
| **W4** | page changes over time | `p = .08` | — | **`changed=1` at seq 9** | ★ fired in its weak form (§3.3) |
| **W5** | non-zero inside `0xf80…0xfff` | `p = .04` | 0 | **0** | ⊘ did not fire |
| **W6** | `gpa_read` refuses | `p = .03` | `READ-REFUSED = 0` | **0** | ⊘ did not fire |
| **C1** | CE `_B` lands below `0xf80` in that page | `p = .70` | `0x440ff00…ff70` | **identical** | ★★★★★ **FIRED, all 8** |
| **C2/C3/C4** | slot collision / other page / not `A,B,PAYLOAD` | `.08/.20/.02` | — | — | ⊘ all refuted |
| P1 | `SEMA-PAGE` dumps | > 0 both | **20** | **20** | ✔ non-vacuous |
| P2 | reader tally | present | **(none)** | **(none)** | ⊘⊘ **FAILED — §3.2** |
| P2b | `READ-REFUSED` | 0 | 0 | 0 | ✔ |
| P3/P3b | `nonzero=` first / last | — | `0/1024` → `0/1024` | `0/1024` → **`12/1024`** | ★★★★★ |
| P4 | `SEMA-PAGE-ZERO` dumps | — | **20** | **1** | ★ the `off` arm never once held a byte |
| P6 | distinct page signatures | — | **1** | **2** | ★ one transition, at `t=+71169ms` |
| P8b | distinct `m0x240` operands | 8 | **8** | **8** | ★★★ `ff00 … ff70`, 16-byte stride, payload `1` |
| P8d | `pbm` runs `/SHORT-` | 0 | **0** | **0** | ✔ nothing truncated by `PROBE_PUSH_BYTES` |
| S13 | `COMPLETION-WATCH → OBSERVED` | **0** | **0** | **0** | ✔ predicted `p=.95` |
| S13b/c | `NOT-OBSERVED` / `last_seen` | 8 / `0x0` | 8 / `0x00000000` | 8 / `0x00000000` | ✔ |
| **S14** | **`CUP2_RC`** | **124 both, movement ZERO**, `p=.93` | **124** | **124** | ✔★ **seventh consecutive predicted zero, seventh measured zero** |
| S15 | `CE-SUBMIT` / `RETIRED` | 0 / 0 | 0/0 | **0/0** | ✔ ⊘ still never printed, ~131 logs |
| R12 | host `Xid` COUNT ⊘ blind | 8 → ? | **8** | **4** | ⚠ **not 0 — §3.1** |
| R12a-d | `Xid` ENGINE / CLIENT / ADDRS / ACCESS | identity, never count | `CE3`,`CE2` / `CE1`,`CE0` / 1 / `VIRT_WRITE` | **`CE3` only / `CE1` only / 1 / `VIRT_WRITE`** | ★★★ **the `CE2`/`CE0` family is GONE** — a substitution a count of 8→4 cannot see |
| S2 | `SEMA-PIN` lines | 0 / >0 | **0** | **12** | ⚠ 12, not 16 — §3.1 |
| S8/S9/S10 | fresh / already / placed | — | 0/0/0 | **1 / 3 / 4** | ⚠ four pins, not eight — §3.1 |
| S11 | negative control refused by name | fires | — | **4** (`NoStatedRun`) | ✔ |
| S11b | `NO PAGE TO PIN` | 0 | 0 | **4** | ⊘⊘ **the risk `w266` said had not materialised, materialising** |
| R7b | `PB-PIN token=` | 16 both | **16** | **16** | ✔ leg-4 guard |
| R6f/R6g | `PT-DECODE` **first** pass | `19615` / `19874` | **`19618`** / `19874` | **`19618`** / `19874` | ⚠ **`bound` moved +3 — §3.4** |
| R6b | `refusals=` | 255 both | 255 | 255 | ✔ carried debt unchanged |
| R15/R16 | `RmInitAdapter failed` / guest `NVRM` | 0 / 31 | 0 / 31 | 0 / 31 | ✔ guest alive both |
| R18 / `adopt=` / `userd=` | `GP_PUT` / GUEST-RING / GUEST-USERD | — | 66 / 16 / 16 | **identical** | ✔ |
| — | `DOORBELL-REFUSED` / heartbeat | 16 / 5 | 16 / 5 | **16 / 5** | ✔ the forwarding plane's refusal is unchanged and still not on the path |

---

## 3. ⊘ WHERE THIS RUN DEVIATES, AND WHERE IT IS WEAKER THAN IT LOOKS

### 3.1 ⊘⊘ THE ORDERING RACE MATERIALISED — `w266` said it had not, and was careful to say only that

`w266` §3.2 recorded *"`NO PAGE TO PIN = 0` means the risk did not materialise, **not** that it
cannot"*. This boot it did: **4** of the 8 CE doorbells arrived while `declared_sites()` was
still empty, so four pins never ran and four channels faulted. That is why `Xid` is **4** and not
the predicted **0**, why `SEMA-PIN` is 12 and not 16, and why `placed_as_asked` is 4 and not 8.

⊘ **This is a real defect, not noise, and it is the next rung's material**: the pin is triggered
by a **CE doorbell** but sourced from a **GR declaration**, and nothing orders those. A CE
channel rung before any GR channel has submitted gets an empty source and a faulting engine.
★★ But read what it did to the *evidence*: a race that cost the rung its clean zero **bought a
per-channel control inside one boot** (§0.1) that neither arm could have produced. The
strongest row in this document exists because the prediction failed.

⚠ **And note the instrument lesson the count would have hidden**: `Xid` went `8 → 4`, and a
reader with only a magnitude would call that *"half fixed"*. The **identity** rows say something
different and sharper — the `ENGINE CE2 / HUBCLIENT_CE0` family is **gone entirely** and only
`CE3 / HUBCLIENT_CE1` remains. `w266`'s own lesson, holding on the first boot after it was
learned.

### 3.2 ⊘⊘ THE TEARDOWN DUMP DOES NOT EXIST, AND THE HARNESS SAID SO RATHER THAN LETTING IT PASS

`PAGE-READER ASSERTION: FAIL — dumps=20 stopped=0`, on **both** arms. `SemaPageReader::close`
prints unconditionally when the observer loop exits, so its absence means **the loop never
exited** — QEMU is powered off without reaching `detach_ram`, so `stop_completion_observer` never
runs. ⇒ There is **no `why=final` dump** and **no reader tally** on either arm.

⊘ Everything in §0 comes from `why=tick` and `why=verdict` dumps taken **while the guest was
still spinning** — which is the more useful instant anyway — so the finding stands. But the
teardown row this rung's design leaned on is **absent**, the assertion caught it, and the fix
belongs to the shutdown path, not to the reader.
★ Recorded as a genuine catch: an instrument that fails loudly on its own missing half is what
the assertion was written for.

### 3.3 ⊘ "The page changed" is ONE transition, and it is bounded by the sample rate

The signature moves exactly once, between `seq=1` (`t=+70893ms`, `declares=1`, all zero) and
`seq=9` (`t=+71169ms`, `declares=8`, 12 non-zero) — a **276 ms** window. It then holds for the
remaining **167 seconds** across 18 dumps.
⊘ So W4 fired only in its weak form: the page changed **once**, and the four writes are not
separable in time by this instrument. A write and an overwrite between two 250 ms samples remain
invisible **by construction** — the dump reports *state*, never *events*, and that is stated in
the reader's own docs.

### 3.4 ⚠ THE CROSS-REVISION GUARD MOVED — `bound` `19615 → 19618`

`unwitnessed=19874` is unchanged, `refusals=255` unchanged, `by-executor=39` unchanged,
`wit=37` unchanged, `PB-PIN=16` unchanged, `GP_PUT=66` unchanged. But the first-pass
`PT-DECODE bound` is **19618**, three higher than `w266`'s `19615`, **identically on both arms**.
⊘ Being identical on both arms means it does not contaminate the arm-to-arm comparison, and
being +3 out of 19 615 (0.015 %) makes a functional cause implausible — the likely reading is
boot-to-boot guest allocation variance, which is corroborated by the semaphore page landing at
`gpa 0x2dc31000` (`off`) and `0x54c6000` (`on`) this run against `0x2197e000` at `w266`.
⚠ **It is not explained, it is bounded.** Any future rung reading `w266`↔`w267` as byte-identical
is wrong.

### 3.5 Other limits, carried

- ⊘ **`[UNCLAIMED]` is what the dump printed for all 12 non-zero slots** and that is correct: this
  device has never decoded a CE `SET_SEMAPHORE` into a watch, so the code cannot name those
  offsets and does not try. The attribution in §0.1 is made **here**, from the pushbuffer decode,
  and is a reading — the instrument refused to guess, and a unit test holds it to that.
- ⊘ **The guest's CPU view is still unmeasured.** `gpa_read` reads the **VMM's** view of the
  memfd. Same memfd, so it is likely the guest sees these 12 words; nothing measures it. Carried
  unpaid from `w266` §4.
- ⊘ **255 `StraddlesLiveBinding`**, **`by-executor=39`**, **host-channel VAS `[NOT MEASURED]`** —
  all untouched.
- ⊘ `pages.len() == 1` again, so `PUSHBUF_MAX_PAGES`, the multi-run coalescer and the
  `LISTING-BOUND` arm are **unexercised** (`P5b = 0` on both) and rest on unit tests.

---

## 4. ⊘ WHAT THIS RUN CANNOT PROVE

- ⊘ **That anything is closer to `CUP2_RC` moving.** `124` on both arms, `CE-SUBMIT → RETIRED`
  still `0`. The plane that started completing is the CE's, and `cuCtxCreate` waits on GR.
- ⊘ **That the four unpinned channels would have written had they been pinned.** Strongly implied
  by the four that were; not measured. The fix is an ordering change and it has not been made.
- ⊘ **That the GR channels would write if their work ran.** No GR work has run — `CE-SUBMIT = 0`,
  `DOORBELL-REFUSED = 16`.
- ⊘ **Anything about ordering finer than 250 ms**, or about writes that did not change the bytes.
- ⊘ **That `bound=19618` is benign.** §3.4 bounds it; it does not explain it.

---

## 5. THE NEXT RUNG

1. ★★★★★ **ORDER THE PIN AGAINST THE DECLARE.** §3.1 is a live, reproducible defect with a
   measured signature (`NO PAGE TO PIN` > 0 ⇒ `Xid` on exactly those channels). The page is one
   page at a fixed VA the guest allocates before any doorbell; the pin does not need to wait for
   a GR declaration to know it. ⊘ But *"pin an address no declaration named"* is precisely the
   `cap2b` class — the fix must derive the page from something the **guest** wrote, not from our
   memory of `0x2_0440f000`.
2. ★★ **Fix the teardown so `close()` runs** (§3.2) — or accept that the observer dies with QEMU
   and delete the `final` arm rather than leave a dump nobody ever gets.
3. ⊘ **Do NOT widen the completion watch to the CE slots.** They are a different engine's
   semaphores and the guest does not wait on them; watching them would produce an `OBSERVED` row
   that means nothing and reads as everything.
4. The unchanged wall: `CE-SUBMIT`. Legs A/B/C are built; nothing submits.
