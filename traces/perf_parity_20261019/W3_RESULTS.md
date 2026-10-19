# W3 synchronous trapped-write analysis

CODE READ: the preregistered cheaper-path test requires arbitrary W1C/set/clear values to take effect before the next guest shadow read. Existing KVM ioeventfd and irqfd do not provide that device-state operation or synchronous barrier; the relevant API references and register path are in `CODE_READ.md`. No equivalent replacement was established, so no write-path behavior was changed.

INFERRED hypothesis, recorded in the private W3 note before inspecting the write census: zero-valued W1C/set/clear aliases might allow a state-preserving no-op shortcut. FALSIFIER: the hot writes are nonzero, stateful operations, or the supposed alias changes observable state.

MEASURED: the mirror baseline DIRECT host trace contains 79,145 CPU_INTR writes and **zero zero-valued writes**. This is the whole trace, including the pre-probe observation period and background work; it is not a per-iteration census. The six dominant offset/value pairs each occur 11,226 times:

| Offset | Value |
|---|---|
| `0xb81408` | `0x101` |
| `0xb81410` | `0xffffffff` |
| `0xb81610` | `0x8` |
| `0xb81208` | `0x101` |
| `0xb81210` | `0xc10007a` |
| `0xb81608` | `0xfffffffe` |

MEASURED: other observed writes include nonzero leaf acknowledgements and additional enable masks. The zero-alias shortcut's observed-workload falsifier holds: it cannot remove these measured exits. This does not exclude a separately designed kernel implementation of the synchronous interrupt-controller state machine; that would require a new design and its own correctness proof.
