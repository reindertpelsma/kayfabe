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
