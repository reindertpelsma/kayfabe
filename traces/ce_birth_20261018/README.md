# CE class-at-birth: pre-code checks

STATUS: PRE-CODE SCOPE STOP, 2026-10-22. No CE product/test changes yet.

CODE READ: diagnosis source is `origin/claude/sa02-diagnosis-20261022`,
`docs/design/V3_SA02_ENERGY_DIAGNOSIS_20261022.md`. Intended branch base is
candidate-6b `baa297752`. Its current `kernel_channel` still ORs the guest-chosen
internal-handle predicate with the RM stamp, and its tree lacks the SA-02 audit
tests. The diagnosis instead requires C1E Windows USER/ADMIN allocations to yield
`kernel_client=false`, while keeping the decoder fix separate from SA-02.
This requires an explicit dependency ruling, not an unannounced authorization change.
Options: keep CE separate and test an SA-02 combination independently; or include
the reviewed SA-02 fix/tests explicitly. Implementation awaits that ruling.

MEASURED (2026-10-22, preserved Q3 runs): [host-checks.json](host-checks.json)
records read-only D1–D3 extraction. Coverage is 11 raw logs (4301–4311), three
exported-only channel censuses (4262/4263/4266), and two absent runs (4264/4267).
D1's complete six-word GP34 segments in 4304/4310 have headers `0x20038090`
(sub4, method `0x240`, count3) and `0x200180c0` (sub4, method `0x300`, count1).
No SET_OBJECT occurs in that segment. Payload is withheld; earlier GP0–33 words
are not retained by these dumps. Three older failure summaries pin NoCeObject,
but their raw segment words are unavailable under Q3.

MEASURED (2026-10-22, D2 fields in [host-checks.json](host-checks.json)): the 11 raw
logs contain 78 COPY2 births, all kernel-stamped Translated. No userspace COPY2
birth appears. Initial token `0x806` retires with forwarded=35 in raw failures
4304/4310 and forwarded=0 in the nine passing raw runs. Other kernel COPY2 work
appears in both passing and failing boots. First-GP0 submission timestamps are
not logged; the later GP34 refusal cannot establish the original trigger order.
INFERRED: the guest-process COPY2 trigger remains unestablished.

MEASURED (2026-10-22, D3 fields in [host-checks.json](host-checks.json)): all root
client and VasKey high-word prefixes in the 11 raw boots are C1D/C1E; all 26
C1E channel facts carry privilege level2. No H-INDIRECT-supporting input was found
in those boots. Five earlier runs lack the raw root/VA census here, so this does
not close H-INDIRECT over every original boot.

CODE READ: the extraction opens existing logs only, publishes headers/methods
and count summaries, and writes no host files. No VM, guest agent or GPU was
invoked. Hardware validation of the proposed fix has n=0. Branch-scope resolution
is required before tests and implementation can satisfy the requested security
contract; there is no tests-green, mutation or Clippy claim for a CE fix yet.
