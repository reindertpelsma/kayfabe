# SA-02 and the Energy NoCeObject failure: diagnosis

**STATUS: LIVE-RESEARCH, 2026-10-22.** This is a GPU-free diagnosis. It used no hardware, changed no
product code, and boots nothing. Labels: **MEASURED** names its run or evidence path. **CODE READ** is a
reading of our source or of public ogkm/UVM source. **INFERRED** is a conclusion from those two. Evidence branches:
`origin/codex/energy-rates-20261022` (`traces/energy_rates_20261018/`, `traces/energy_bisect_20261018/`,
`traces/energy_bisect_20261022/`), `origin/codex/qualify-cand6-20261022`, and the review
`origin/claude/review-cand6-20261022:docs/audits/2026-10-22-candidate6-review.md`.

## 0. Verdict

**SA-02 is not the mechanism.** The best-supported root cause is a **pre-existing latent decoder defect**.
SA-02 does not expose it through any code path we can find.

- CODE READ: arm A (`40583bc2`) differs from arm D (`468a35c9`) in product code only by two pure predicates
  in `crates/kf-rm/src/chanlink.rs`: `kernel_channel` drops the handle-range OR, and `is_rm_internal_client`
  narrows the mask from `0xC1E00000` to `0xFFF00000`. Neither predicate feeds the T-mode decoder state.
- The failing channel's birth gives the same answer under both predicates. MEASURED in
  `traces/energy_rates_20261018/failure-census.csv` (runs 4262/4263/4266/4304/4310): client `0xc1d00019`,
  `internalFlags=0x16` = KERNEL, COPY2. Q2's evaluation of the old and new predicates over every CHAN-FACTS
  record (`traces/energy_bisect_20261018/channel-comparison.json`) found no difference.
- The death is `kf_chan::tmode`'s `NoCeObject`. A Translated ring on a **dedicated copy-engine runlist** has
  no CE tier until the guest sends a CE-class `SET_OBJECT`. The Windows kernel driver sends an engine
  semaphore on subchannel 4 without one. This is the same pattern that killed the Windows kernel NVDEC and
  NVENC channels on 2026-10-11 and 2026-10-14, before SA-02. Video rings were fixed by seeding their tier at
  birth (`1c503f87`); copy rings never got that seed.
- **Why the failure is intermittent:** in every passing run the channel forwards 0 entries. The defect fires
  only when the guest actually uses its COPY2 kernel channel. Which workload event triggers that use is
  **not established** (§6, D2).
- **The fix is in the decoder, not in SA-02** (§4). Keep SA-02 as implemented.
- **The open question is the association:** SA-02 builds failed 5/8 and non-SA-02 builds 0/8. It stays
  unexplained (§3.3) until a D or B run is seen using the channel (§6).

## 1. PR #73 against master, end to end (CODE READ)

`git diff origin/master...origin/codex/audit-fixes-sa02-20261022` touches `crates/kf-rm/src/chanlink.rs`
(+19/−26), two new test files and a traces README. `git diff 468a35c9 40583bc2 -- crates` (D→A) is the
same `chanlink.rs` change plus the tests. Nothing in `kf-chan`, `kf-qemu` or `kf-host` changes.

| Use site | Old | New | Effect on a Windows birth with client `0xc1d…` and stamp `0x16` |
|---|---|---|---|
| `kernel_channel(h, p)` → `ChannelAlloc.kernel_client` (route; `tspace::Privileged`) | `internal(h) ∥ p.is_kernel()` | `p.is_kernel()` | none: `p.is_kernel()` is true |
| `is_rm_internal_client(h)` → `UserWorkFacts.rm_internal`, `windows_kernel_pid` exclusion | `h & 0xC1E00000 == 0xC1E00000` | `h & 0xFFF00000 == 0xC1E00000` | none: `0xc1d…` is false under both |
| `is_rm_internal_client(client)` → `mem.rs::kernel_vas_for` (initial `TwinState`) | same mask change | same | none: the failing VasKey is `0xc1d00002_ff000870`, false under both |
| `chan.rs::kernel_by` and the `root:` log lines | same mask change | same | log text only |

CODE READ: across the whole `0xC1D00000..=0xC1DFFFFF` range, the old mask matches nothing (checked
exhaustively in Python on 2026-10-22). The two masks differ only for handles outside `0xC1E00000..=0xC1EFFFFF`
that still carry the bits of `0xC1E00000`, such as `0xC1F…`, `0xE1E…` and `0xFF…`. A channel born with
stamp KERNEL is kernel under both predicates.

MEASURED, 2026-10-22, by grep over every committed trace (324 files carry `client … root:` lines, for example
`traces/windows_code43_walls_20261007/run53-probe-window.log`): every root client starts with `0xc1d` or
`0xc1e`, with 2313 + 20 `0xc1e` lines and 910 + 548 `0xc1d` lines. All 281 committed `0xc1e…` CHAN-FACTS
records, Windows runs 60/64/65/66 included, carry `privilege level 2` (KERNEL). Within the committed corpus,
then, no input exists on which the old and new predicates disagree.

## 2. Failing life against passing life (evidence paths as named)

Same birth in every run, failing or passing. MEASURED in `traces/energy_rates_20261018/43{01..11}/channels.json`
and `traces/energy_bisect_20261018/channel-comparison-excerpts.txt` (runs 3454/4260/4262–4267, 2026-10-22):

`CHAN-FACTS 0xc1d00019:0xff040004 parent=0xff0e0000 (TSG) class=0xc56f engine=0xb (COPY2) flags=0x200620
ctxShare=0 vas=0xc1d00002:ff000870 kernel=true privilege=level 2 internalFlags=0x16 ProcessID=0x308/0x30c/0x310`,
born `Translated`, `kernel_by=PRIVILEGE=KERNEL` (no `rm-internal`), token `0x806` → host `0x1002d`.
`NV50_DEFERRED_API` is admitted on the channel, and a software classID is reported.

| Life | Runs | Retire line |
|---|---|---|
| failing | 4260 (mixed), 4262 C1, 4263 A1, 4266 A2, 4304 C3, 4310 C4 | `forwarded=35 submissions=1 last_put=35 … dead=Rewrite{gp:34, NoCeObject{subch:4, method:0x240}}`, then `REFUSED-AND-POISONED (§7)` and guest nvlddmkm 153 ×3 |
| passing, same channel | 3454, 4264, 4267, 4301–4303, 4305–4309, 4311 | `forwarded=0 submissions=0 dead=None` |
| after the reset (failing runs) | for example run 4304, qemu line 21114 | re-born `0xc1d000f6:0xff040004`, `ProcessID=4`, Translated, `forwarded=0` |

INFERRED: the passing runs never submit on this channel. Nothing in the evidence shows what a D or B build
would do with the channel's first 35 entries, because no D or B run ever produced them.

## 3. Hypotheses, each with the falsifier written before its check

| # | Hypothesis | Falsifier (stated first) | Result |
|---|---|---|---|
| H-CLASS | SA-02 classed this channel differently: user/Passthrough or a non-kernel T-space | The birth line reads `Translated` with `kernel_by=PRIVILEGE=KERNEL` in a failing SA-02 run, and both predicates return true for `(0xc1d00019, 0x16)` | **Falsified.** MEASURED in runs 4263/4304 birth lines (`4304/channels.json` line 1668). CODE READ: §1 table |
| H-RACE | The stamp is recorded after classification, or the channel is born before its parent's stamp | `privilege` is decoded from the same RmAlloc params in the same `respond` call that builds `ChannelAlloc` (no cross-message state) | **Falsified.** CODE READ: `chanlink.rs` decodes `privilege` from `params` and calls `kernel_channel(h.client, privilege)` in the same function. No stamp is carried from a parent |
| H-LAZY | A different birth path, such as an RM-internal CE channel allocated lazily under load, carries no stamp | The failing channel's facts equal the passing runs' facts, and it was born at boot | **Falsified.** MEASURED: same CHAN-FACTS and boot-time line (~1660) in failing and passing runs; it is the same channel, used or unused |
| H-INDIRECT | SA-02 changes some other object's route, VA-space class or reply, and that alters what the guest does | A committed or Q3 input exists on which old and new predicates disagree: a root, VA space or channel with a handle outside `0xc1d/0xc1e`, or a `0xc1e` channel with no KERNEL stamp | **Not falsified, but unsupported.** No such input in the committed corpus (§1). For Q3 the root and VAS lines are not public; the cheapest check is D3 in §6 |
| H-LATENT | The decoder needs an explicit CE `SET_OBJECT`; the Windows kernel driver's engine-sync segment issues `SET_SEMAPHORE_A/B/PAYLOAD` on subchannel 4 without one. The defect is independent of SA-02 | (a) The refused segment, or an earlier entry of that channel, holds a CE-class `SET_OBJECT` header (subchannel 4, method 0, then `0xC7B5`). (b) A D or B run uses `0x806` (`forwarded>0`) and survives | **Not falsified.** CODE READ: `tmode.rs` sets `st.tier` only in `HostMethod::SetObject`, and the CE path then does `st.tier.ok_or(NoCeObject)`. The video tier is seeded at birth (`GrConfig.video`); no CE equivalent exists. The birth source is byte-identical across arms (`traces/energy_bisect_20261022/source-path-check.json`). MEASURED, 2026-10-22, scratch test (§3.1) |

### 3.1 Scratch check of the H-LATENT mechanism (GPU-free, 2026-10-22)

On 2026-10-22, at rev `4f6cf6e19` (the decoder is byte-identical in D and A), the scratch file
`crates/kf-chan/tests/zz_sa02_scratch.rs` was run under the shared build lock (`cargo test -p kf-chan`) and
then deleted; it was never committed. Results: 3 passed. A CE engine-semaphore segment (`SET_SEMAPHORE_A/B/PAYLOAD`,
then a semaphore-only `LAUNCH_DMA`, all on subchannel 4, no `SET_OBJECT`) is refused `NoCeObject{subch:4,
method:0x240}` on a default `TState`. The same words decode to one `Ir::Launch` (tier `C7b5`, `CeSema{bytes:4}`,
no src/dst) when `st.tier` is preset to `C7b5`, the AD104 host CE class. A `SET_OBJECT 0xC797` (GR class) on
subchannel 4 is still refused `ForeignClass`. The words are synthetic, built from public C7B5 fields; they are
not the guest's bytes (see D1).

```rust
fn segment() -> Vec<u32> {           // header via kf_abi::submit::method_header_inc
    let mut w = packet(4, 0x240, &[0x1, 0x2050_f060, 0x1]);         // SET_SEMAPHORE_A/B/PAYLOAD
    w.extend(packet(4, 0x300, &[(1 << 2) | (1 << 3) | (2 << 5)]));   // LAUNCH_DMA: NONE, flush, release, non-blocking intr
    w
}
// TState::default()            -> Err(NoCeObject { subch: 4, method: 0x240 })
// st.tier = Some(Tier::C7b5)   -> Ok([Launch(CeLaunch { sub: 4, tier: C7b5, sema: Some(..), src: None, dst: None, .. })])
// seeded + SET_OBJECT 0xc797   -> Err(ForeignClass { subch: 4, class: 0xc797 })
```

### 3.2 The precedent

MEASURED in `traces/win11_userring_20261014/README.md` §1 (run 2990 at `0c5fedff`, 2026-10-14): the Windows
kernel NVDEC channel's first segment held host semaphores, then `NVC9B0_SEMAPHORE_A/B/C` at `0x240–0x248` on
subchannel 4, with no `SET_OBJECT`. It was refused with the identical `NoCeObject{subch:4, method:0x240}`, then
DEAD, then TDR. MEASURED in `traces/windows_matrix_20261011/encoder-session/windows-focus/qemu-116{0,1,2}-kernel-video-refusal.txt`
(rev `a61af1c3`, 2026-10-11): the same signature on the kernel NVENC channel, followed by nvlddmkm 153 ×3.
After `1c503f87`, the README's run records the kernel NVDEC ring retiring `forwarded=35/70/105, dead=None`.
The count of 35 entries per use matches this channel's `forwarded=35`.

CODE READ, public source: `kernel-open/nvidia-uvm/uvm_turing_ce.c` (ogkm 595.84) describes its CE `SET_OBJECT`
as a check that UVM chose to make: it uses subchannel 0 to verify the engine type. `uvm_hopper_sec2.c` says
SEC2 "is not guaranteed to support the SET_OBJECT method, so we shouldn't submit it". RM's own CE pushes,
in `channel_utils.c` and `mem_utils_gm107.c` (open 580.65.06), do send `SET_OBJECT` in every block.
INFERRED: an engine accepts its own methods without `SET_OBJECT`, so the kernel driver's omission is legal on
hardware. The closed Windows driver's stream itself has not been read (RE is forbidden), and none of this is
verified on bare metal.

### 3.3 The association with SA-02

MEASURED counts (`traces/energy_rates_20261018/rates.md`, 2026-10-22): D 0/4, B 0/4, A 2/4, C 3/4. In the
interleaved Q3 series alone: A 0/2, C 2/2, D 0/4, B 0/3. INFERRED arithmetic: one-sided Fisher on the pooled
5/8 against 0/8 gives p ≈ 0.013. On Q3 alone (2 of 4 against 0 of 7), p ≈ 0.11. Three of the five failures come
from the earlier block (4262–4267), which was not interleaved.

INFERRED: H-CLASS, H-RACE and H-LAZY are falsified, and H-INDIRECT has no supporting input, so no code path
connects SA-02 to whether the guest uses `0x806`. The association is either chance or an unmeasured confound,
such as host state, run order or guest workload timing. **The prediction that falsifies this
verdict:** a D or B run in which `0x806` forwards more than 0 entries and survives (`dead=None`). Under
H-LATENT, any such run dies with the identical signature.

## 4. Decision and corrected design

**SA-02 as implemented is correct for this birth path. Do not weaken it.** The stamp already covers the
kernel driver's COPY2 channel (`0x16` = KERNEL). The guest-chosen-handle denial stays as it is.

Two residuals, both CODE READ and both fail-safe:
- `kernel_vas_for` still starts a `0xC1E…` client's VA space as `Kernel`, as the review notes. A guest user who
  picks such a handle gets a twin that refuses every Passthrough birth (`UserInKernelSpace`). Its Translated
  birth needs the stamp, which SA-02 now requires. The result is self-harm only.
- `rm_internal` in `UserWorkFacts` can only keep a channel off Passthrough.

Leave both unchanged here. Changing `kernel_vas_for` affects when the scrubber's privileged leaves are placed,
so it needs its own Linux hardware gate.

**The fix, in the decoder (new work, its own branch, not part of SA-02):** seed the CE tier at a Translated
birth on a **dedicated copy-engine runlist** from the class of the CE object the **host** channel holds since
birth. That object is `kf_chan::host::on_engine_layout` → `alloc_ce_object`, class
`classes.ce_object().ce_object_id()`. This mirrors the video tier.
1. `kf_chan::tmode::GrConfig` gains `ce: u32` (0 = none). `TranslatedRing::set_gr` sets
   `st.tier = Tier::of_ce_class(ce)` when `ce != 0` and `video == 0`.
2. The `kf-qemu` Translated birth (`chan.rs`, beside `video_class`) passes the host CE class **only** when the
   ring's engine is a COPYn (`copy_index_of_engine_type(engine).is_some()`). It never does so for GRAPHICS:
   subchannel 4 there is GRCE, and nothing shows that binding to be implicit. It never does so for video.
   Factor this choice into a pure function, `birth_decoder_config(engine, video_class, host_ce_class, gr_tier)`,
   so that tests reach the real decision.
3. A seeded ring may re-bind only its own host class. A `SET_OBJECT` naming any other CE class (for example
   `0xC8B5`, which has a wider upper mask) is refused `ForeignClass`. The guest cannot choose a tier wider than
   the engine's.
4. Security (CODE READ): the seed grants nothing the guest could not already obtain by sending `SET_OBJECT`
   itself. Every CE method still passes the tier's tables. Semaphore and copy addresses are still bound
   through the channel's rows by the perimeter. The seed comes from host state, never from guest bytes.

## 5. Tests and mutants for the fix (proposed; written on the fix branch, tests first)

- **T1, EXPECTED RED:** `kf-chan/tests/copy_ring_birth_seed.rs::a_copy_runlist_kernel_ring_admits_the_engine_semaphore_without_set_object`.
  It builds a `TranslatedRing::new_tmode` with the config from `birth_decoder_config(0xb, 0, 0xC7B5, false)`
  and feeds GP 0..34 through a fake `GuestMemory`. The words are D1's measured words where they are public
  structure (headers and methods); otherwise the synthetic words of §3.1. The test expects `Next::Bind`
  with a CE semaphore and no guest VA word emitted. It is RED today: the refusal is `NoCeObject{4,0x240}` at GP 34.
- **T1b, census replay:** a `ChannelPolicy` (`GuestOs::Windows`, user-work flag on) receives the root
  `0xc1d00019` (ProcessID `0x310`), then the channel alloc with the measured facts (§2). It asserts
  `kernel_client`, `!user_work`, `user_ring_permit=None` and the COPY2 engine. Expected GREEN under both
  predicates; this pins H-CLASS (extends Q2's `rpc_characterization.rs`).
- **T2, control:** a GRAPHICS-runlist T-mode ring stays unseeded. A CE method on subchannel 4 before
  `SET_OBJECT` still gives `NoCeObject`.
- **T3, control:** a video ring is unchanged; `video` wins over `ce`.
- **T4, hostile:**
  - On a seeded ring, `SET_OBJECT 0xC8B5` is refused `ForeignClass`, and so is `SET_OBJECT 0xC797`.
  - A seeded semaphore whose address is outside the VM's rows is refused at bind.
  - A physical `LAUNCH_DMA` operand still takes the existing translated-operand rule.
- **T5, provenance:** the guest allocates a CE object of class `0xC8B5` under the channel
  (`ChanStatement::EngineObject`), and the tier stays `C7b5`. The seed is host-sourced.
- **SA-02 regressions stay green:** `audit_kernel_channel_stamp.rs`, plus one new Windows case. A
  `GuestOs::Windows` root `0xc1e0_0100` with a USER or ADMIN channel stamp gives `kernel_client=false`.
- **Mutants:**

  | Mutant | Test that must fail |
  |---|---|
  | M1: remove the seed | T1 |
  | M2: seed every ring, GRAPHICS included | T2 |
  | M3: seed from the guest-declared CE class | T5 |
  | M4: accept any CE class on a seeded ring | T4 |
  | M5: seed when `video != 0` | T3 |
  | M6: restore `internal(h) ∥ stamp` in `kernel_channel` | the existing SA-02 tests, plus the new Windows case |

Hardware gates for the fix, run in order:
1. `v3_gates.sh`, then the fast suite (30/30).
2. Energy-04 standalone on **D + fix** and on **C + fix**, at least 4 each.
3. Pass criterion: no `NoCeObject` and no guest 153. Any `0x806` life with `forwarded>0` must show `dead=None`.
4. Edge/NVENC regressions.

## 6. Missing measurements, cheapest first

- **D1 (no code; preserved logs on the trusted host):** the line `kf3: ring REFUSED segment gp 34 va … first words:`
  is already printed by `ring.rs::refused_segment`. Grep it in the raw qemu logs of runs
  4262/4263/4266/4304/4310 under `/data/codex-Q3/`.
  - It falsifies H-LATENT(a) if a `0x2001_8000`-form header (subchannel 4, method 0) appears before `0x240`.
  - It refines T1 if the remaining methods are named.
  - Publish headers and methods only, with payload withheld as the lanes already do.
- **D2 (no code):** this explains the trigger. In each of the 16 runs, list user and kernel births with
  `engine=Some(b)` (COPY2) and their RETIRED `forwarded` counts. Note which qemu lines precede the first
  `0x806` submission.
  - Hypothesis H-TRIG: a guest process's COPY2 work causes the kernel driver's per-engine sync on `0x806`.
  - Falsifier: failing runs show no COPY2 activity before GP 0 of `0x806`, or passing runs show the same activity.
- **D3 (no code):** this closes H-INDIRECT. List the distinct prefixes of the Q3 runs' `client … root:` lines
  and of every VasKey's high word.
  - Falsifier of "SA-02 is inert on these boots": any handle outside `0xc1d/0xc1e`.
  - Also any `0xc1e` channel whose privilege is not level 2.
- **D4 (default-off code, only if D1 is insufficient):** `KF3_DEBUG_TMODE_HISTORY=1`.
  - Per T-mode ring, a fixed 64-slot ring buffer of decoded `(gp, subch, method, count)` headers, with no data words.
  - Dumped once at the ring's first refusal.
  - Bounded memory; no behaviour change; off in the production profile.
- **D5 (bench, decisive for the association):** 8 more standalone D runs.
  - Under §3.3, all of them are at risk; D's 0/4 so far has probability ≈ 0.69^4 ≈ 0.23 at the pooled rate 5/16.
  - Prediction: any D run that uses `0x806` dies with the identical signature.

## 7. What this does not show

The 35 entries of the failing submission are unmeasured beyond their count. The kernel driver's reason for
using COPY2 is unmeasured. The hardware semantics of an implicit CE binding are inferred from public UVM
comments and the video precedent, not verified on bare metal.
