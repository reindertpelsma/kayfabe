# The GA10x work-submit token — the encoding, and which instrument settled it

**Status: BUILT and MEASURED, 2026-08-01.** Increment **E3** of
`execution_plane_increments.md`. `Ga10xArch::decode_doorbell` answered `None` before this
and answers a `(runlist, chid)` pair now.

Every claim below carries a label and they are used strictly:

| label | meaning |
|---|---|
| **MEASURED** | observed on real hardware, or produced by NVIDIA's own compiled code, with the output quoted and the artifact named |
| **SOURCE** | read out of `research_clones/ogkm-580.159.04/`, cited to file:line. Not run. |
| **INFERRED** | follows from a MEASURED fact plus a SOURCE fact, with the step written out |
| **UNKNOWN** | nobody knows, and this file says so rather than guessing |

---

## 0. Why this increment was pulled ahead of E2, and why it needed instruments at all

`execution_plane_increments.md` §2.1:

> **E3 — the doorbell token decode — is the riskiest, because it is the only increment
> whose wrong answer is silent.** A wrong decode does not fail; it **routes a guest's ring
> to another channel**. On the Mode-2 path we are the GSP, so there is no second party to
> notice.

and both of the project's standing oracles are structurally blind to it:

- **The mock cannot catch it.** `MockArch::token_for` is *the inverse of the mock's own
  decode* — an invented encoding round-tripping against itself. The test that did this is
  now called `mock_arch_token_roundtrip_is_self_consistent_only` and its doc comment says
  what it is not. This exact shape let a planted mutation survive earlier the same day.
- **The C artifact cannot catch it.** `c_rust_trace_differential.md` records that the
  **completion plane has NO C oracle** — the C forges completions — and that
  forwarding-mode traces are non-hermetic by construction. A green differential across a
  doorbell says nothing about where the ring went.

So the encoding had to be established against instruments that cannot be satisfied by our
own beliefs, while the bench and the passing `rm-ladder` still exist.

---

## 1. The encoding

**Two fields, and RM's encoder writes nothing else.**

| field | bits | meaning |
|---|---|---|
| `NV_CTRL_VF_DOORBELL_VECTOR` | **11:0** | the channel id (`chId`) |
| `NV_CTRL_VF_DOORBELL_RUNLIST_ID` | **22:16** | the runlist id |
| — | 15:12, 31:23, 63:32 | **unwritable**: RM starts from `val = 0` and sets only the two above |

**SOURCE.** `kfifoGenerateWorkSubmitTokenHal_GA100`
(`ogkm-580: src/nvidia/src/kernel/gpu/fifo/arch/ampere/kernel_fifo_ga100.c`):

```c
    // Here we construct token to be a concatenation of runlist id and channel id
    val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _RUNLIST_ID, runlistId, val);
    val = FLD_SET_DRF_NUM(_CTRL, _VF_DOORBELL, _VECTOR,     chId,      val);
```

with the field positions at
`ogkm-580: src/common/inc/swref/published/ampere/ga100/dev_ctrl.h:26-27`, and GA106 bound
to that `_GA100` implementation by the driver's own dispatch table
(`g_kernel_fifo_nvoc.c:649-652`, `/* ChipHal: GA100 | GA102 | GA103 | GA104 | GA106 | … */`).

⊘ **None of the above is why the decoder is trusted.** It is a *reading*, and a transcribed
reading is precisely the failure `isolate_the_drivers_own_checks` names. §2 and §3 are the
reasons.

---

## 2. Instrument A — RM's own encoder, compiled and swept  (this is what SETTLED it)

**MEASURED.** `tests/oracle/worksubmit_token_oracle.c` compiles
`kfifoGenerateWorkSubmitTokenHal_GA100` **itself** — sliced byte-for-byte out of the file
the driver's own dispatch table names, with the impl file's own `#include` lines carried
into the slice so `NV_CTRL_VF_DOORBELL_*` reaches the compiler through the driver's headers
and not through a constant this repository typed. `kchannelIsRunlistSet` /
`kchannelSetRunlistSet` are sliced too, so the "is this channel on a runlist yet" gate is
the driver's own predicate over the driver's own state bits. Eight stubs, none of them
arithmetic. Nothing is vendored: `tests/build.rs` hands the compiler absolute paths and
refuses loudly when the checkout is absent.

★ **This is the half hardware could not do.** The RTX 3060 produced chids 4–9 and upper
values 0, 1, 2, 8 — small numbers that a decoder whose mask is several bits too wide agrees
with everywhere. The encoder can be *swept*:

```
case chid_bit11    runlist=0   chid=2048        token=0x00000800
case chid_bit12    runlist=0   chid=4096        token=0x00000000   ← DROPPED
case runlist_bit6  runlist=64  chid=0           token=0x00400000
case runlist_bit7  runlist=128 chid=0           token=0x00000000   ← DROPPED
case both_max      runlist=127 chid=4095        token=0x007f0fff
case both_overflow runlist=0xffffffff chid=0xffffffff token=0x007f0fff
case unbound       runlist=3   chid=11          status=0x40  token=-
```

So: chid bits 0–11 and runlist bits 0–6 survive, everything past each field's end is
dropped, and the widest token RM can emit is exactly `0x007f_0fff`.
`our_decode_inverts_rms_own_encoder_over_the_whole_field_space` differentials every case;
`a_token_rm_could_not_have_written_is_refused` derives the unwritable mask **from the
oracle's own saturation result** and requires `decode_doorbell` to refuse every bit in it —
so the refusal rule is the encoder's, not a plausibility rule we invented.

`case unbound` is the other half: a channel that was never bound to a runlist gets
`NV_ERR_INVALID_STATE` (`0x40`) and **no token**. That is the fact the whole `R13` rung
leans on when it calls a token "evidence".

---

## 3. Instrument B — a real GA106, asked without going near a token

**MEASURED.** `docs/reference/bench_evidence/doorbell-census-ba74151.out` — RTX 3060 /
GA106, host 580.159.04 **open**, stock DKMS module (the capture records `instrumented
symbols in .ko: 0`, `in kallsyms: 0`), rung `R13c`, built at `ba74151` with the revision
stamped into the binary at compile time.

### 3.1 ⊘ The obvious source was disqualified first

`R13`/`R13b` already print `(runlist N chid M)` beside every token — computed as
`token >> 16` and `token & 0xFFFF`. **That is the token restated**, and reading it as
agreement measures nothing. (It is also, incidentally, the wrong mask: `0xFFFF` on both
halves rather than 7 and 12 bits.) The archived `rm-ladder-419afe8.out` is exactly that,
and §4 records what it cost to read it as evidence.

### 3.2 What was asked instead

`NV2080_CTRL_CMD_FIFO_GET_ALLOCATED_CHANNELS` takes a **runlistId as input** and returns a
bitmask of allocated chids, walked out of that runlist's `CHID_MGR` by
`kfifoGetAllocatedChannelMask_IMPL` (`ogkm-580: kernel_fifo.c:3371-3443`) — RM's own channel
allocator, with no work-submit token anywhere on the path. Snapshot, allocate one channel,
snapshot again; the bit that appeared is the chid RM just assigned.

★★ **The before/after pair is the instrument, not the second snapshot.** A bitmask read
once cannot distinguish *"this channel is at chid 7"* from *"some channel is at chid 7"* —
the boolean-witness failure — and the bench box has other channels on it. Any allocation
whose diff is not exactly one bit is printed as `SAMPLE-AMBIGUOUS`, which carries no
`chid=` and is therefore unusable as evidence by construction.

```
SAMPLE engine_type=0x1 token=0x00000004 chid=4 chid_namespace=0
SAMPLE engine_type=0x9 token=0x00000005 chid=5 chid_namespace=0
SAMPLE engine_type=0xa token=0x00000006 chid=6 chid_namespace=0
SAMPLE engine_type=0xb token=0x00010007 chid=7 chid_namespace=0
SAMPLE engine_type=0xc token=0x00020008 chid=8 chid_namespace=0
SAMPLE engine_type=0xd token=0x00080009 chid=9 chid_namespace=0
```

Six live channels, six distinct chids, and `token & 0xFFF == chid` in every one — while a
second, disjoint field at bit 16 takes four different values. Fed those same
`(runlist, chid)` pairs, the §2 oracle reproduces all six tokens byte for byte.

### 3.3 ⚠ The first capture pinned NOTHING, and the reason is worth keeping

Its rung allocated **and freed** one channel per engine type, so RM handed the same
recycled chid back every time and the census read `chid=4` six times. One chid value
exercises no width and no shift. Holding the six channels **simultaneously** is what
produced the spread above, and `ga106_hardware_tokens_decode_to_rms_own_chids` asserts
`chids.len() >= 6` so the fix cannot quietly regress.

### 3.4 ⊘ What this box could NOT be asked: the runlist IDs

The only control that names a runlist id per engine,
`NV2080_CTRL_CMD_FIFO_GET_DEVICE_INFO_TABLE` (`engineData[ENGINE_INFO_TYPE_RUNLIST]`), is
`KERNEL_PRIVILEGED` — `flags = 0x5c040`, neither `PRIVILEGED` (0x4) nor `NON_PRIVILEGED`
(0x8) (`g_subdevice_nvoc.c:4996`, `control.h:170-208`) — and is refused to every usermode
client including root. The census **asks anyway and records the answer**:

```
FACT device_info_table=Err(InsufficientPermissions)
```

which is the difference between *"we did not measure the runlist id"* and *"we asked and
were told no"*.

A second attempt, `CHANNEL_GROUPS_IN_USE_PER_ENGINE` (non-privileged), was meant to recover
the engine→runlist **partition** without the ids. It is **vacuous on this part**, and the
rung says so itself (`FACT partition_is_vacuous=1`): with `per_runlist_channel_ram=0`,
`kfifoGetChidMgr` returns `ppChidMgr[0]` for every runlist id
(`ogkm-580: kernel_fifo.c:1457-1466`), so the per-engine count is one global number and
every engine reads as a member of every class. The `PARTITION` lines are in the artifact
and measure nothing; they are kept because a reader who sees them needs the verdict line
next to them.

### 3.5 So what did hardware settle, exactly

**MEASURED** — run `doorbell-census-ba74151.out`, RTX 3060 / GA106 / 580.159.04, rev
`ba74151`, 2026-08-01, replayed by `tests/tests/doorbell_token.rs`: the token's low 12
bits are the chid RM's allocator assigned, across six distinct values, on six
simultaneously live channels — and a second field exists above bit 15 that varies with the
engine type and is not the chid.

**INFERRED** (this step, written out): RM's encoder is **total and injective** over its two
fields — it starts from zero and writes only `RUNLIST_ID` and `VECTOR` (§2, swept, not
read) — so any token RM emits determines `(runlistId, chId)` uniquely by inversion. The
low half of that inversion is confirmed against hardware; therefore the upper field of a
hardware token *is* `runlistId`.

⊘ **No hardware reading of the runlist field's identity exists** — this box could not be
asked for one (§3.4), so there is no measurement of it and this document does not pretend
otherwise. The identity rests on instrument A plus the inference above. Getting it directly
needs a privileged reader (an instrumented module, or BAR0), which nothing here has.

⚠ A note on the sentence above, because it cost two `scripts/claim_ledger.py --gate` cycles
on 2026-08-01. It was first written as a
denial in the form *"… is not <strong-word> on <the machine>"*, and `claim_ledger.py`'s
`bare_hardware_claim` rule scored it as a **bare hardware claim**: that rule matches the
strong word within thirty characters of the machine noun and — unlike `classify`, whose
comment says the honest-downgrade check deliberately runs first — never consults
`HONEST_RE`, so a sentence *denying* a measurement reads to it exactly like one asserting
one. Then the note quoting the offending phrase tripped the same rule again.

The **wording** was changed both times, never the rule. A bar moved to let one's own diff
through is precisely the failure this ledger exists to catch; the rule's blind spot belongs
in a document, not in a lowered ceiling.

---

## 4. ★★★ The correction: what an earlier reading of the archive got wrong

The first version of this work asserted, in `RunlistId`'s and `DoorbellTarget`'s doc
comments, that run `rm-ladder-419afe8.out:21-25` (RTX 3060 / GA106, rev `419afe8`)
**measured** five copy-engine channels holding chid 7 on runlists 0, 1, 2 and 8 — and
therefore that `(GpuId, VChid)` could not be a channel identity and the core's exec-plane
index was aliasing channels.

**That was wrong, and the census refutes it.** Those five channels were allocated and freed
one at a time; RM recycled the same chid. The archive shows *handle reuse*, and nothing in
it distinguishes reuse from simultaneity — the same class of mistake as reading a
single-snapshot bitmask as an attribution.

**MEASURED** instead: `FACT per_runlist_channel_ram=0` and
`FACT chid_namespaces=[0] of 0..24`. On GA106 there is **one global `CHID_MGR`**; six
channels across four runlists took six distinct chids. So on this part a chid is
device-unique and `(GpuId, VChid)` **is** a channel identity.

⚠ **That is a fact about the part, not about the key.** `kfifoIsPerRunlistChramEnabled` is
a runtime property (`kernel_fifo_init.c:195-231`); where it is true, chids are per-runlist
and two live channels can share one. `DoorbellTarget::runlist` and `DoorbellRoute::runlist`
carry the value today so that a future `(GpuId, RunlistId, VChid)` key has something to be
built from and so that the **decoder** is never the thing that lost it.
`the_census_records_the_part_the_conclusion_is_scoped_to` pins this scope to the evidence
file: a capture taken on a part where the flag is 1 turns it red and forces the conclusion
to be re-read.

---

## 5. Does a wrong decode still fail silently?

Partly. Honestly:

| a decode that is wrong about… | now fails | how |
|---|---|---|
| the **width** of either field | **loudly, in CI-adjacent tests** | the §2 sweep sets every bit of both fields, including the first bit past each end |
| the **shift** of either field | **loudly** | same sweep, plus six hardware tokens whose chids are 4–9 |
| **reserved bits** (accepting a token RM could not emit) | **loudly** | `a_token_rm_could_not_have_written_is_refused`, over a mask derived from the oracle's own saturation |
| **collapsing distinct channels** to one target | **loudly** | `distinct_channels_decode_to_distinct_targets` over six real channels |
| the **VF/SR-IOV** rewrite of `chId` | ⊘ **still silent** | the oracle drives `gfid = 0`; nothing here exercises `kfifoGetVChIdForSChId` |
| routing a well-formed token to the **wrong live channel** | ⊘ **not this seam** | that is `kayfabe_core`'s exec-plane index; a miss is `FwdFault::UnknownVchid`, a *stale hit* is not caught anywhere yet |
| **any other generation** | ⊘ **still silent** | `Ad10xArch`/`Gh100Arch` delegate to `MockArch`'s invented encoding; Turing binds `_TU102` and Blackwell `_GB202`/`_GB100`, none differentialled |

⊘ And the thing E3 does **not** touch at all: whether the decoded channel's ring is one the
guest was entitled to submit on. That is the #14 ring gate in `plan_doorbell`, unchanged.

---

## 5.1 The bite check — and the number that is uncomfortable

**MEASURED**, `scripts/bite_doorbell_token.py` at rev `b2ff418` on 2026-08-01, 38-core box,
17 planted defects in `Ga10xArch::decode_doorbell`, three arms run per bite:

```
15/15 live bites caught by the E3 guards (11 by the ORACLE alone, 0 by HARDWARE alone).
 0/15 caught by the pre-existing mock suite.
 2 rows are EQUIVALENT MUTANTS (required to stay green; 0 did not).
```

Three readings, and the middle one is the useful one:

1. **`0/15` for the mock suite.** §0's claim that `MockArch::token_for` is structurally
   blind to this class was an *assertion* until this ran. It is now a number. The mock arm
   is in the harness for exactly that reason — a harness that ran only the two new arms
   would have left the argument unquantified.
2. ⚠ **`0` bites caught by HARDWARE alone.** Every defect the census caught, the oracle
   caught too. So instrument B bought **no additional mutation-catching power**, and it
   would be dishonest to imply otherwise. What it buys is different and not measurable this
   way: the oracle is told `(runlist, chid)` and asked what token results — it assumes the
   pairing. Only the census shows that on a real part the low field really is the number
   *RM's allocator* handed out, over six live channels. Instrument A settles the
   **encoding**; instrument B settles that the encoding is the one this **part** uses.
   Delete the census and every bite still fails — and the decoder would rest entirely on a
   compiled model of a driver, never on a GPU.
3. ★★★ **Two rows are equivalent mutants, and finding that out changed the harness rather
   than the code.** Widening the chid mask to 16 bits, or the runlist mask to 16, cannot
   change any answer: the reserved-bit refusal rejects every token with a bit in 15:12 or
   31:23 *before* the masks are read, so on all 2^24 inputs the decoder accepts the wide and
   narrow masks agree (checked exhaustively). They were first reported as `MISSED BY
   EVERYTHING`, which reads as a hole and is not one. Rows 14 and 15 relax the refusal *and*
   widen the field together, and both are caught — so the widths are load-bearing the moment
   the refusal is gone, and the redundancy is defence in depth rather than dead code.

## 6. What is committed, and where

| artifact | what it is |
|---|---|
| `crates/kayfabe-chips/src/ga10x.rs` | `Ga10xArch::decode_doorbell` |
| `crates/kayfabe-arch/src/lib.rs` | `DoorbellTarget { vchid, runlist }` |
| `crates/kayfabe-arch/src/ids.rs` | `RunlistId` |
| `tests/oracle/worksubmit_token_oracle.c` + `tests/build.rs` | instrument A, HAL binding derived from the driver's dispatch table |
| `tests/tests/worksubmit_token_oracle.rs` | the sweep differential (`TOKEN-ORACLE-GATE:` census lines) |
| `crates/kayfabe-isolate-host/src/bin/rmladder.rs` | rung `R13c`, `--doorbell-census` |
| `docs/reference/bench_evidence/doorbell-census-ba74151.out` | instrument B's output, with its own revision stamp |
| `tests/tests/doorbell_token.rs` | the hardware differential, keyed on that file |
| `scripts/bite_doorbell_token.py` | the three-arm bite check, with the equivalent-mutant class |
| `.github/workflows/ci.yml` | the `TOKEN-ORACLE-GATE` reached-count step, floor 2 |
