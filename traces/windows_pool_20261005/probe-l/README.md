# Windows L: TMO capability constructor experiment

**STATUS: RESEARCH, 2026-10-05.** Code 43 remains. This deliberately restricted
experiment cannot provide functioning display channels or establish Windows GPU
workload success.

Product and runner: `b431aeaf9fca5d78b451754c0de7b8dbbe9d7652`, branch
`codex/windows-tmo-constructor-probe-20261005`, based on J's `fdc991c9`. Fresh
Windows 580.88 overlay, guest ABI 580.65.06, native Linux/open 595.91.07 on
RTX 4070 AD104. All J flags remain enabled; the new independent
`--display-tmo-constructor-probe` sets the generated TMO capability and pairs it
with immutable refusal of every display method before decoding/enqueueing,
assembly mutation, arming or completion. The normal mode remains default.

This is an explicit diagnostic deviation from ordinary capability semantics,
not tone-mapping support. A cap-only change would be misleading because the
current engine otherwise stores unsupported in-range methods and could complete
work without applying its color transform. The blanket refusal prevents that
outcome. Existing VMM display-context and boot-image activity is outside that
method-refusal claim. The [implementation contract](https://github.com/reindertpelsma/kayfabe/blob/b431aeaf/tools/display-tmo-constructor-probe/README.md)
records the source-derived driver/family cells and limits.

There are 365 traced RPCs and 367 serviced messages, versus J's 363/365. The
multiset difference is only two extra `0x50700117` cleanup controls. GPU-channel
births and display methods remain zero. The guard is therefore not exercised by
a hardware submission in this run; its behavior is covered by the unit tests.
No unsupported TMO method executed or completed.

The new signed KD analysis completed successfully. The reviewed NVIDIA journal
and exact-driver control flow show that **all three preceding allocation helpers
now passed**, including J's failing TMO-buffer helper. The new leaf assertion is
RVA `0x16e97f4`: a later per-window display constructor returned with its success
flag clear. The precise failed constructor check or storage allocation is not yet
identified. See the [causal analysis](../../../tools/windows-debug-capture/evidence/2026-10-05-startdevice-j.md).
This is stronger evidence of progress than RPC counts alone, but it does not
establish a production TMO implementation or every possible earlier failure.

Windows still reports Code 43 and NVIDIA-SMI exit 9. It shut down through
pinned-key SSH and QEMU exited 0. The physical GPU remained bound to the Linux
NVIDIA driver and available afterward. Raw dump/journal/debugger output are
preserved privately on the controller. Only reviewed file-relative assertion
addresses and ordinary bench text are committed here.

Validation before this run: 92 display-library tests, QEMU library compilation,
Python syntax and runner help passed; independent review found no blocking issue
for the isolated experiment. Full hostile-isolation/hardware merge gates remain
outstanding, and no production merge is implied.
