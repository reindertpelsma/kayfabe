# V3 Windows support profile: one switch for the Windows-only capabilities

**STATUS: DESIGN-ONLY, 2026-10-13 (owner decision on the default; scope to be fixed by ablation).** Related: `V3_WINDOWS_USER_RING_RELAY.md`, `V3_FLAG_INVENTORY.md`, owner rulings D4 (Windows-only behaviour keyed on the guest's declared state, not a flag) and §AE (no heuristics; a fallback is a clean mode), §AI (Linux Passthrough forbids host-owned mappings).

## Decision (owner, 2026-10-13)
A device property `windows-support=on|off` (name provisional) gates a chunk of kayfabe that Linux guests do not use. **Default `on`, so every image is supported out of the box.** `off` is a hardened Linux-only profile: the Windows capabilities cannot be reached even by a guest that declares a Windows identity.

## What the switch is (and is not)
- It gates AVAILABILITY of capabilities. The guest's declared state still selects routing inside an enabled profile (D4); the switch is not a heuristic and not a behaviour fork with one wrong value: both values are valid configurations.
- A configuration chosen by the VM's launcher (a device property), never an environment variable and never inferred from the guest.
- Mechanism: a single capability witness constructed only when the switch is on (the pattern already used by `WindowsUserRingPermit` and the Windows mapping permit), passed to the admission points, so with the switch off the Windows paths cannot be entered. No scattered `if windows` checks.

## Candidate scope (to confirm by ablation: a feature is Windows-only only if a Linux guest runs fully with it off)
USER ring relay and migratable passthrough; the deferred API (`5080` admission, registry, host-authored actions); Windows user-channel passthrough routing; the Windows-only display features (SDR colour, LUT mirror, ILUT offset 256, IMP enable) and display-control answers only Windows issues. Not gated: anything Linux needs (UVM, doorbell tiers, the common display path, channel budget, kf-mem).

## Tests (the point of the switch)
- Linux gates (9/9 + gate 10) and the fast suite (30/30) pass with `windows-support=off` AND with `on`; the off run is the proof that Windows additions do not change Linux.
- Windows runs use `on`. A Windows guest on an `off` VM must fail cleanly and by name (counted refusals), never hang or fault.
- Hostile test: a Linux-profile guest that declares a Windows identity and sends Windows-only allocations is refused with `windows-support=off`.
- The flag inventory records this property as a profile, and every behaviour flag it replaces is deleted (hardwired or removed), per the flag rules: only debug and measurement trace flags remain.

## Per-driver and per-die differences are derived, not flagged (owner, 2026-10-13)
Where a behaviour is right for one guest driver or die and wrong for another, it is DERIVED inside kayfabe from the guest's declared state (the driver branch and version from SET_GUEST_SYSTEM_INFO, the host-derived chip facts), so it never needs to appear on a command line. Conditions for a derived switch to be accepted:
- it changes COMPATIBILITY behaviour only, never a security boundary: a guest controls what it declares, so it must be safe for any declared value (a lie can only change that guest's own compatibility behaviour, never privilege, mapping exposure or another VM);
- it is a pure function of declared and host-derived facts, covered by GPU-free tests that include unknown and malformed values;
- a declared state outside the measured set takes the clean fallback (refuse by name, count, claim nothing; §AH), never a neighbour's behaviour;
- a flag that survives ablation only because two drivers disagree is replaced by such a derivation, and the flag is deleted.

## The host driver is measured; the guest version is untrusted (owner, 2026-10-13)
Two disjoint version axes (`kf_abi::host_driver`, `kf_abi::hostabi`, `V3_DRIVER_MATRIX.md`): the GUEST version (declared, untrusted) only selects how kayfabe decodes the guest's own requests, so a false declaration can at most make that guest's requests fail or mis-parse; the HOST driver version is measured on the host
by kayfabe, which no guest can influence, and it alone selects the layouts of every host ioctl kayfabe issues. Host calls are authored from kayfabe's own internal structs and carried to the host's measured layout by field name (exact-membership rule: an unmeasured host driver, or a field the host layout lacks that carries a value, is refused by name), never forwarded as guest bytes.
Properties to pin with tests (GPU-free): (1) the host version used for layout selection comes only from the host query, never from any guest-supplied value; (2) for every host control kayfabe issues, a property test with random guest-controlled input shows that only the allowlisted value fields can differ from the authored default (handles, pointers and every unnamed field are authored, translated or zero, never taken from guest bytes); (3) a host driver outside the measured set refuses the calls whose layout was not measured, with a counted named error;
(4) a guest that declares any version, including a different one from what it really is, cannot change a host struct field outside that allowlist. Open: the `0x2080a028`/legacy-GSS host asks have no measured layout at 595.91.07 and are refused (inventory row `hostask-2080a028`).

## Guest version: detected, not trusted; skew rejected when it cannot be satisfied (owner, 2026-10-13)
- **No vGPU presentation** (`V3_VGPU_COMPAT_RESEARCH_20261013.md`): every vGPU graphics workload also runs on bare metal, which kayfabe supports, so no workload needs it.
- **Detect the guest version instead of relying only on `guest-driver=` on the command line.** Sources, all guest-controlled and therefore never trusted: the fn-1 fields (`guestDriverVersion`, `guestVersion`, changelist, vgx pair) and the GSP firmware image the guest loads (its version section), checked against each other. `guest-driver=` stays only as an optional pin: if given it must equal the detection or the boot is refused by name. What the detection selects is a layout row; a false declaration can only select a row that the guest's own requests then fail to match (refused, counted), never another VM's or the host's behaviour.
- **Skew handling is ours, tested, and bounded.** For every pair (declared guest branch, presented row) kayfabe supports only what the matrix shows to be losslessly transcodable for the payloads it uses (fields added or removed that kayfabe does not need are fine; anything else is a refusal named at boot), the way vGPU also refuses what it cannot satisfy. Closed-driver branches that have no open tag (Windows r580_78) map to a measured twin row only with evidence (captured exchanges plus the oracle), recorded per row.
- **Tests:** the fn-1 negotiation oracle built from NVIDIA's own code (`claude/fn1-oracle-20261013`); matrix transcoding round trips across every supported pair; declared-version fuzz (random and malformed strings refuse or fall back cleanly); a mismatch beyond the supported distance refuses at boot with a named error.
