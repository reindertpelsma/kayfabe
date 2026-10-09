# VFIO refusal ablation: does kayfabe's refusal list cause the Windows crash?

**STATUS: LIVE, 2026-10-09 (in progress — this file is updated after each boot).**

Branch `claude/vfio-refusal-ablation-20261009` (from `claude/vfio-dvi-reference-20261008`). Host
172.22.1.20 (trusted). Mechanism and evidence under `tools/vfio-gsp-observer/` (the shared,
device-agnostic observer; this ablation's `x-gsp-refuse` property lives there, DEBUG ONLY,
perturbing, default off) and `traces/vfio_refusal_ablation_20261009/` (this directory: the
refusal lists, the run table, the harness copies used on the host, and `patch/` — the diff of
this session's changes against the `tools/vfio-gsp-observer` this branch started from).

## Owner's idea and falsifier

Make the **real GSP** on the VFIO reference refuse exactly the requests kayfabe refuses, and see
whether Windows still works. Falsifier, stated before the first boot: *"with kayfabe's refusals
the VFIO guest still shows the lock screen >= 3 minutes idle AND survives the gesture (no
VIDEO_TDR_TIMEOUT_DETECTED 0x117 live dump, no driver reset) in at least 2 of 3 boots."* If that
holds, the refusals are exonerated and VFIO traces become a 1:1 comparison for everything else
kayfabe diverges on; if the guest dies like kayfabe (TDR 0x117 ~7 s after a lock-screen drag, or
earlier), the refusals are the cause and can be bisected.

## Mechanism (measured, not assumed)

`tools/vfio-gsp-observer/qemu/gsp-observer.c`, property `x-gsp-refuse=FILE` on the patched
vfio-pci device (requires `x-gsp-observer`). At the command-queue doorbell write — **after** the
observer has recorded the guest's original request and **before** the write reaches the GPU —
`vg_refuse_scan()` (`core/observer.c`) walks every not-yet-consumed queue element; for each whose
`(fn, cmd)` (fn 76 GSP_RM_CONTROL) or `(fn, hClass)` (fn 103 GSP_RM_ALLOC) matches a rule, it
rewrites that field to an unused id of the same shape (`vg_refuse_default_key()`: same class bits,
command/class index moved into the `0xff00-0xffff` band) and fixes the element's checksum exactly
as `message_queue_cpu.c`'s `_checkSum32` computes it (XOR of the 32-bit words, zero when valid).
The real GSP then answers the (now-invalid) request itself, with its own real error. Every rewrite
is logged once per `(fn,key)` to the QEMU log with a running counter; `gsp.jsonl` (the observer
capture) records **both** the guest's original request and the status the guest actually
received, so every rewrite's effect is checked against ground truth, never assumed.

A rule file has one `FN KEY [NEWKEY [obj]]` line per refusal (`#` comments). `obj` (fn 76 only)
rewrites `hObject` instead of `cmd` — kept for the record (see Results) but **not used** in the
final list: it works, but gives a different status (`0x57`, not kayfabe's `0x56`) and is not 100%
reliable (a handful of replies still came back status 0 even with the field rewritten).

**Verification tool:** `tools/verify_rewrites.py GSP_JSONL REFUSE_FILE`. It does **not** pair a
reply back to its own request by submission order (unreliable once the observer's capture drops
anything — `sequence_gaps` in its footer — because one dropped message shifts every later FIFO
pairing, see *Corrections* below). Instead it classifies every **reply** directly by the field the
GSP echoes back (the rewritten `cmd`/`hClass` for a `field 0` rule — chosen unused, so nothing
else collides with it; the original `cmd` for an `obj` rule, since every occurrence of that cmd is
rewritten for the life of the rule). It reports, per rule, how many replies carry status 0
(**ineffective** — the guest was not refused) vs. each non-zero status seen, and exits 1 with a
loud banner if any rule shows status 0 even once. This is wired into
`tools/ablate_session.sh`, so every boot checks its own rewrites automatically.

⊘ **Correction (folded in above the text it corrects):** the first reading of boot `t1` (below)
called `0x20809004`'s rewrite ineffective. That was a measurement error — reading the VRPC
header's `rpc_result` (which stays 0 for a GSP_RM_CONTROL reply either way) instead of the reply
**body**'s own `status` field (offset +12, after the 32-byte VRPC header; fn 103's body status is
at +16). The first version of `verify_rewrites.py` had the offsets right but paired replies to
requests by naive per-function FIFO order, which silently miscounted under capture gaps. The
version in this tree classifies by echoed field instead (above) and was cross-checked against an
independent reading of the same captures before being trusted for the final list.

## Refusal list

`refusal-full.txt`, 58 rules, recomputed independently from the kayfabe observer streams of runs
114/118/152 (`boundary-kayfabe-{114,118,152}/gsp.jsonl`) with `tools/make_refusal_list.py`: every
`(fn, key)` whose reply in a kayfabe run carries a non-zero VRPC `rpc_result` or body `status`.
fn 71 (`CONTINUATION_RECORD`, confirmed from ogkm's `rpc_global_enums.h`) is **excluded**: it is a
fragment of an oversized fn 76/103 call, not a request with a `cmd`/`class` of its own to rewrite,
and `tools/vfio-gsp-observer` has no mechanism for it yet — documented gap, not blocking the main
experiment. fn 103 `hClass 0xc56f` (status `0x40`) is excluded per the owner's note (a different
cause). All 58 rules use the plain default rewrite (empty `NEWKEY`/`obj` field); no per-family
customisation was needed in the end (see *Candidate rewrites measured* below).

## Candidate rewrites measured (`t1`, `probe2`)

| key (sample) | mechanism | result |
|---|---|---|
| `0x20809004` | default (flip to `0xff04`, bit `0x8000` SET) | **refused**, status `0x56`, 0 ineffective (t1: 472/479; probe2, different boot: 483/491) |
| `0x20809004`, `0x2080852e` | explicit bit-`0x8000`-CLEAR id (`0x7f04`/`0x7e04`) | refused, status `0x56`, 0 ineffective — works too, but adds no value over the default |
| `0x2080012f`, `0x20800a3a`, `0x800106`, `0x730285` (non-"GSS-legacy" controls) | default | refused, status `0x56`, 0 ineffective |
| `0x2080b201`, `0x2080a618` | `obj` (hObject -> NULL) | refused **mostly** (62/63, 53/56) but status `0x57` (not kayfabe's `0x56`) and not 100% reliable — **dropped** |
| `0x402c` (fn 103 alloc) | default | refused, but status `0x22`, not `0x56` — **documented difference**, not pursued further (an alloc's invalid-class error is a different NV_STATUS than a refused control's; the brief allows documenting rather than chasing the exact value) |

**Conclusion:** there is no "GSS-legacy leniency" that defeats a cmd-id rewrite — the plain
default mechanism (unmodified `vg_refuse_default_key()`) already refuses every family tested,
including the ones with the `0x8000` bit set in their original `cmd`. The earlier appearance of
leniency was the header/body measurement bug above.

## Run table

| boot | flags | idle result | gesture/interaction result | time of death | events | notes |
|---|---|---|---|---|---|---|
| `t1` | 1 rule (`0x20809004`, default) | n/a (not the falsifier run) | owner manually signed in + ran `nvidia-smi`; desktop reached | — | — | reference/diagnostic boot to measure one rewrite's real-hardware status; not a survival test |
| `probe2` | 9 rules (mixed mechanisms, see above) | n/a | owner manually interacted (sign-in); desktop reached | — | — | diagnostic boot to compare rewrite mechanisms per family |
| `idle1` | 58 rules (full list, default rewrite) | **driver dead at load, before the lock screen**: GSP stream ends 14:52:19.85Z (observer +7.0 s), guest nvlddmkm id=14 at 14:52:18.95, 14+153 at 14:52:24.4, LogonUI first seen 14:52:29; NVIDIA device Code 43 (coordinator's read), Basic Display Adapter Code 10 | owner input attached ~5 min later; no NVIDIA display to use | driver failure ~19 s after QEMU start, ~10 s before the lock screen | nvlddmkm 14/153 only; no TDR 0x117, no LiveDump, no UnloadingGuestDriver | verify_rewrites: 58/58 refused, 0 ineffective; QEMU counters rewritten=76 verify_failed=0 header_bad=0 seq_gaps=0 |
| `bisA` | 49 rules (58 minus the 9 keys kayfabe also answers OK) | **not completed: host 172.22.1.20 became unreachable (ssh/ping timeout) about a minute after the launch; result unknown** | | | | falsifier stated before the run: A is still Code 43 / nvlddmkm 14+153 at boot; falsified if Code 0 and nvidia-smi exit 0 three minutes after LogonUI |

*(further rows added as the idle-only and gesture boots run; see `runs/` on the host for the raw
evidence — `command.json`, `gsp.jsonl`, `trace.log`, `verify.txt`, `guestlogs/` per boot. Evidence
copied into this directory is filtered: no secrets, no owner home/Scaleway IPs.)*

## Measured vs inferred

- **Measured:** the rewrite mechanism (checksum-correct, hardware-verified via `verify_rewrites.py`
  against `gsp.jsonl`'s ground truth) refuses every sampled family with kayfabe's own status
  `0x56` for controls (`0x22` for the one alloc sampled, a different but real NV_STATUS).
- **Measured, corrected:** `t1`'s single-rule rewrite was effective, not ineffective (see
  *Correction* above) — the owner's manual sign-in on `t1`/`probe2` succeeded on real hardware
  with those specific refusals active.
- **Measured (idle1):** with all 58 refusals the real-hardware guest never gets a working NVIDIA driver: it fails at load (nvlddmkm 14/153), before the lock screen and before any input, so the falsifier's first clause (lock screen with a live driver for >= 3 min) is NOT met. First divergence from the unablated VFIO reference (boot3) at request 60: after the refused fn76 0x20802a0f the reference sends 0x20802a06/0x20802a0d, the ablated guest does not (kayfabe's guest does not either); then a ~850-iteration retry loop over 0x730108/0x731152/0x731341/0x731140/0x73117a/0x730282, then 0xc3700104 and 0x73029a, then silence.
- **Inferred, not yet measured (which refusals matter):** the 58-rule set is sufficient to kill the driver at load; which subset is responsible is the bisect (49-rule boot A pending, host unreachable). 9 of the 58 are also answered OK by kayfabe at least once (coordinator's overshoot analysis), so the 58 overshoot kayfabe.
- **Inferred, not yet measured:** whether the full 58-rule refusal set, held for 3+ idle minutes
  and/or a scripted gesture with no manual interaction, reproduces kayfabe's TDR 0x117 — this is
  the actual experiment the falsifier is about, pending the `idle1`/gesture boots below.
- fn 71's exclusion means this ablation's refusal set is not bit-for-bit identical to kayfabe's
  (kayfabe also refuses 262-286 fn 71 continuation fragments per run); if the guest survives
  anyway, that is still informative (the 58 rewritable refusals are exonerated); if it dies, fn 71
  remains an open variable that this experiment does not rule in or out.

Trailers: `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`,
`Claude-Session: https://claude.ai/code/session_01BsKBVkrPunx1N6x6AoegFZ`.
