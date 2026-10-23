# Dedicated-copy class at birth

STATUS: RESEARCH, 2026-10-22. Base candidate 6, commit 60655d661.
Claude night authority includes SA-02; the CE change is independently revertible.

INFERRED hypothesis, stated before tests: the host CE object's class at a
dedicated-copy birth supplies the missing decoder tier. FALSIFIER: the real ring
still refuses the GP 34 semaphore; graphics/video controls widen; a guest CE
class changes the host seed; address binding or Windows C1E denial fails.

MEASURED pre-code checks, 2026-10-22, read-only Q3 retained logs:
[host-checks.json](host-checks.json) carries source-relative paths, log hashes,
line numbers, headers, birth/retirement counts, and coverage for D1–D3. Payload
words are withheld. No VM, GPU, guest agent, or remote write was used.

MEASURED D1, 2026-10-22, runs 4304/4310, qemu.log lines 18794/17080: GP 34 has
0x20038090 (subchannel 4, method 0x240, count 3), then 0x200180c0
(subchannel 4, method 0x300, count 1), and no SET_OBJECT in that segment.
Earlier GP 0..33 words are not retained. Runs 4262/4263/4266 have exported
censuses only; their segment headers cannot be checked. The test replays public
headers with synthetic payloads and 34 synthetic NOP entries, through real
TranslatedRing::new_tmode, GuestMemory, decoder, and bind.

MEASURED D2, 2026-10-22, host-checks.json: 11 raw runs 4301–4311, 3 older exported censuses,
2 absent runs 4264/4267. All 78 raw COPY2 births are kernel-stamped Translated;
none are user COPY2 births. Initial 0x806 forwards 35 in 4304/4310 and 0 in the
other 9 raw runs. Other kernel COPY2 work is present in passing and failing
runs. No timestamp for the first GP 0 submission survives.
INFERRED: the user-work trigger remains unresolved.

MEASURED D3, 2026-10-22, host-checks.json: all root/VasKey prefixes in the 11 raw logs
are C1D/C1E; all 26 C1E CHAN-FACTS are KERNEL level 2.
INFERRED: no supporting H-INDIRECT input in this retained subset; missing
older raw logs prevent an all 16-run conclusion.

CODE READ: the host allocates its CE object using HostRm.classes.ce_object(),
independently of guest EngineObject statements. The QEMU Translated birth must
use that host fact and must restrict the seed to dedicated COPYn engines,
excluding graphics and video. SA-02 remains unchanged.

MEASURED tests-first replay at commit 5d2bc402b, tests/copy_ring_birth_seed.rs:
6 passed, 9 failed by assertion; the primary failure is NoCeObject at GP 34,
subchannel 4, method 0x240. The new CE config API does not exist in this red
commit, so the fixture uses the current birth inputs. The fix changes only
that input adapter to the production pure choice; assertions stay unchanged.

INFERRED hardware hypothesis: candidate 6 + CE passes the Energy rate runs,
whose previous C arm failed 3/4 (traces/energy_rates_20261018/rates.md).
FALSIFIER: any NoCeObject, guest 153, or active 0x806 life dead!=None.
Hardware n=0 here: v3_gates, fast suite 30/30, at least 4 Energy runs per arm,
D+CE comparison, and Edge/NVENC are pending. Implicit CE hardware binding
semantics and the workload trigger remain unverified.

CODE READ at commit 6f823a88a: QEMU obtains the seed via host.ce_class(),
which returns the field initialized from rm.ce_class_id(); alloc_ce_object uses
the same host class selection. Only dedicated COPYn and non-video births get
that seed. The class pin covers hardware subchannels and CE binds on software
subchannels, while existing deferred/software objects retain their path.
No SA-02 predicate or kernel_vas_for change is included.

MEASURED focused tests at commit 6f823a88a, 2026-10-22:
copy_ring_birth_seed: 15 passed; copy_birth_policy: 5 passed. Before the fix,
the two tests-only EXPECTED RED commits had 9 passed / 11 assertion failures.
Both Windows C1E USER/ADMIN denials passed before and after. The input adapters
now call birth_decoder_config; no assertion was loosened.

CODE READ test boundary: the guest-object cases send real ChannelPolicy RPCs
and replay a real ring with an independent host-class input. They do not create
a physical HostRing or execute ChanPlane's host allocation path. The host-source
wiring above is source analysis; hardware acceptance is pending.

MEASURED mutation campaign at commit 6f823a88a, 2026-10-22:
[mutants.json](mutants.json) lists every distinct assertion killer and the
restored source hashes. Every variant compiled and ran; no missing target or
setup error counts as a kill. All mutations were restored byte-for-byte, and
the complete changed-crate/security suites run on that restored source.

| Source mutation | Distinct assertion killers |
|---|---:|
| M1 remove birth seed | 6 |
| M2 seed graphics | 2 |
| M3 substitute guest C8B5 for the host-class input | 2 |
| M4 accept foreign CE binds | 3 |
| M5 seed despite a video class | 2 |
| M6 restore handle OR stamp authorization | 2 |

CODE READ scope of M3: the injected C8B5 value stands in for a guest-declared
class at the pure selector. The RPC/ring tests kill that wrong-source value;
this does not exercise physical host allocation or a live ChanPlane.

MEASURED Clippy --all-targets, 2026-10-22: master at e9c6d39e5, candidate
at 60655d661, and fix at 6f823a88a each have 273 identical source-bound warning
fingerprints across kf-chan/kf-qemu/kf-rm and their dependencies. Added 0 versus
both bases. QEMU deny lints and both debt ledgers are unchanged.

MEASURED final restored suites at commit 6f823a88a, 2026-10-22:
`cargo test --release --locked -j2 -p kf-chan -p kf-qemu -p kf-rm`,
with KAYFABE_NO_KVM=1, nice19, own target and the global flock, completed
97 Cargo result summaries: 1822 passed, 0 failed, 4 ignored.
[validation.json](validation.json) records counts, existing ignores, and the
log hash. Unit, integration, and doc-test summaries are included. Every
ignored case already exists at candidate 6; none was added or loosened.
Existing audit_kernel_channel_stamp and secaudit_internal_client_range
regressions are included, alongside both new Windows denial cases.

CODE READ limits: no test assertion was loosened. The new-API fixture adapters
now call the production pure choice, the existing graphics full-field config
adds ce=0, and the output helper uses as_chunks to satisfy Clippy. No unsafe
perimeter, launcher, timer, privilege predicate, or debt ledger was changed.

MEASURED final claims gate, 2026-10-22: `python3 scripts/ci/debt.py claims`
reports existing-debt=3756, new=0, resolved=2. No new debt was recorded.
