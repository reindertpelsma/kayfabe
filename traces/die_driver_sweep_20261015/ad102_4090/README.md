# AD102 native Windows reference wave

**STATUS: LIVE, 2026-10-15.** Instance56096336, RTX4090/AD102, auto-retire
about15:40:55UTC. Working branch codex/sweep-ada-20261015; no acceptance claim yet.

The owner requires both Linux and Windows guest suites across several supported
builds before a family is covered. A native reference does not satisfy a guest cell.
GA104 Windows remains unresolved; Turing and Blackwell hardware remain unverified.

## Order and hypotheses, before the run

Order change reported to the owner: allocate this rental's first useful window to
the stock Windows oracle. GA104 conversion started with only44minutes remaining
and never regained SSH; that is an installation timeout, not a GPU/driver limit.
This run allows more installation time and preserves the original175-minute timer.

H1: the public, pinned windows-on-vast installer can establish a stock Windows11
EnterpriseLTSC2024/580.88 oracle on this AD102 rental. Falsifier: no verified SSH,
installer completion and NVIDIA adapter Code0 before the lease deadline. A timeout
falsifies provisioning on this run, not AD102 support.

H2: the bounded measurement module can read the physical display caps page and
capture GSP initialization after explicit GSP enablement. Falsifier: module fails
to load, page read rejects, no queue attach during a bounded restart, or a lost
capture prefix prevents attributing the requested controls. No T3 row will be
added without a successful stock Windows read keyed to this die and build.

H3: the cross-built Windows census decoder reproduces its golden fixture result
on native Windows. Falsifier: runtime failure or output mismatch. Raw captures
remain on the untrusted rental; only scrubbed, explicitly selected text returns.

Public installer: reindertpelsma/windows-on-vast93b65b3347601f35f4146602ff85950e389b409f;
upstream bin456789/reinstall80c3d5e175f39c2d2bbd267cd583842140154140.
Measurement module source32973658a; census decoder4ff31a249. Their CPU/build
checks are recorded on their topic branches; runtime remains unverified here.
