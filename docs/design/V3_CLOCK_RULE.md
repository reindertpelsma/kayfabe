# V3: the guest clock rule (network decides; a recorded clock is never earlier than loaded software)

STATUS: LIVE, 2026-11-16. Owner rulings of 2026-11-16 (`docs/OWNER_RULINGS.md` §AU), implemented GPU-free in
`scripts/bench/windows/clockrule.py`, used by `scripts/bench/windows/win_vm.sh`, tested by `scripts/ci/test_clockrule.py` and
`scripts/ci/clockrule_mutants.py`. No VM was started for this change: two hardware confirmations are listed at the end.
Labels: MEASURED = a command run on 2026-11-16 (named); CODE READ = read in a named file, not run on a guest; DESIGN INTENT =
decided, not built.

## Clock rule

The clock depends only on whether the guest has network.

| network | slirp | `-rtc` | why |
|---|---|---|---|
| off | `restrict=on` (only the ssh forward) | `base=<recorded>,clock=vm,driftfix=slew` | deterministic |
| on | `restrict=off` | `base=utc,driftfix=slew` | the internet needs valid time |

An offline image needs no current time; a moving clock only adds ways for it to break (expiring certificates and caches, timers,
update prompts). A fixed clock makes a run reproducible, the way pinning dependency versions does; reproducible-build practice
pins the timestamp for the same reason. A guest with network needs the real time. `clock=vm` makes the recorded clock advance
monotonically from its start value, independent of the host clock (CODE READ: QEMU documentation; no guest run yet).

The rule is the same for open and proprietary images. The one code path is `clockrule.py` (stdlib only; shell launchers only call it).

## Where the recorded time comes from

- The build writes a sidecar `clock.json` next to the image (`disk/clock.json` for `win_vm.sh`): the guest's own UTC time at the last
  probe before the build's shutdown (`win_vm.sh install`: after the check boot's probes), the source, the tool revision, the build's
  loaded software (name and `created_utc`) and which of them set the floor (`floor_input`, or null). Command: `clockrule.py record`.
- The install and check boots run the real clock (`--phase build`): they are the boots that make the record.
- A proprietary image keeps exactly the clock of its `state.json` (`propimage.py` `boot_clock`). No floor and no `created_utc` lookup
  apply to it, because nothing is loaded into it (owner, 2026-11-16). `propimage.py` was left unchanged: its mutant harness copies
  that one file into a temporary tree, so importing the shared helper there is a follow-up (below).

## The floor: a recorded clock is never earlier than the creation time of software loaded into the image

`clock_floor = max(recorded, max(created_utc of every software loaded) + margin)`, margin 1 h by default (`--margin-s`). The
effective offline clock is the floor. With no software loaded the floor is the recorded clock.

- **Only software counts**: a package installed or loaded into a non-proprietary image that boots offline with a recorded clock
  (the SPECviewperf bundle on the open LTSC image, a guest driver installer). Our scripts and test/input assets (repo scripts,
  harness files, the open test-assets bundle, data files, media such as the app disk) do not count and are never passed to the helper.
- **Where `created_utc` comes from.** The pin: an item with `loads_software: true` and `created_utc` (next to its sha256 and size).
  For a private R2 object the source is the object's R2 `LastModified`. The stage passes `loaded_software` explicitly
  (`WINVM_LOADED_SOFTWARE`, with `PATH=CREATED_UTC` or pins in `WINVM_CLOCK_PINS`), not every staged file.
- **mtime is a fallback, not a source.** A file's mtime is not reliable after a copy or rsync without `-t`. Without a `created_utc`
  the helper uses the mtime and prints `KF_CLOCK_INPUT_NO_CREATED input=<name> using=mtime` (verdict WARN).
- **Refusal `KF_CLOCK_BEFORE_INPUT`.** An explicit recorded clock (`WINVM_RTC_BASE`) earlier than the creation of a loaded software
  is refused, and so is a `record` whose guest time is earlier than a build input. A sidecar clock that is merely older than newly
  loaded software is raised to the floor, not refused.
- Two different `created_utc` values for one software in the pins are refused (`bad_pin`).

## Network and strictness

- Online with a recorded clock is always refused (`online_recorded_clock`).
- Offline on the real clock: with no `clock.json` (older or mid-build images) the launcher falls back to the real clock, prints one
  line `KF_CLOCK_NO_RECORD image=<name> using=real` and writes verdict WARN to `logs/clock_verdict.json`. `WINVM_CLOCK_STRICT=1`
  refuses it instead (`offline_real_clock_strict`); the owner can flip the default later. The repo has no env-read allowlist lint on
  `next` yet (`V3_FLAG_INVENTORY.md`, stage 2-4 not done); the knob is documented in the `win_vm.sh` header and
  `scripts/bench/windows/README.md`, the place the other `WINVM_*` knobs live, and must be added to that lint when it lands.
- A corrupt `clock.json` is refused (`bad_record`), never read as "no record".

## What was checked

- MEASURED 2026-11-16 (GPU-free, no VM): `scripts/ci/test_clockrule.py` (57 tests: the matrix {offline, online} x {record, none} x
  {strict, lenient}, the floor with newer, older, equal, missing-created and many inputs, `win_vm.sh` dry-run argv) and
  `scripts/ci/clockrule_mutants.py` (26 mutants, each failing at least two tests, one of them named).
- CODE READ only: that QEMU starts a guest at `base=` with `clock=vm`; that the guest's `Get-Date` equals the recorded time at boot.

## Needs hardware confirmation (not done; no VM was booted)

1. One open-image (LTSC) boot offline with the recorded clock: the guest's UTC time at the first probe is at or after the sidecar's
   time (and after the floor when software is loaded), and `logs/clock_verdict.json` says PASS.
2. One online boot with the real clock: the guest's UTC time matches the host's.

## Follow-ups (not in this change)

- Wire `propimage.py` to the shared assert (`clockrule.assert_rule`): its mutant harness needs `clockrule.py` copied into its temporary tree.
- Add `loads_software` and `created_utc` to the pins of software loaded into open images: `scripts/bench/box/specviewperf.pin` (a text
  pin parsed by a `sed` in `specviewperf_stage.sh`, not changed here; the bundle was made on the pc, its time is the owner's to state)
  and the NVIDIA installer in `win_vm.sh`'s `MEDIA`. `scripts/ci/propimage_apps_pins.json` is the proprietary image's and carries none.
- Pass `WINVM_LOADED_SOFTWARE` from the stages that load software, and `clockrule.py record --input` after a driver install.
