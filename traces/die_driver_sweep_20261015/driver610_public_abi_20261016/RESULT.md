**STATUS: ANSWERED, 2026-10-16.** B4/A3 public ABI prerequisite for Windows610.88.

MEASURED public source: NVIDIA's610.57.04 nvBldVer.h names Windows610.88/r610_85-151 at changelist38566155 and Linux610.57.04 at38610010. NVIDIA's release notes pair those releases; that is a release-family relationship, not exact ABI identity. See public-build-facts.json for primary URLs and header SHA.

MEASURED CPU probe: the current product rejects this identity with TwinRefusal::ChangelistsDiffer even when guestClNum is0. Probe exits0 only when this exact refusal is observed. No version override, hardware boot or neighbor-layout substitution was attempted. Thus a second Windows610.88 acceptance cell remains UNRUN/unsupported exact public identity in the current product. Do not count the previous transient Windows Update install as suite coverage.

INFERRED next: select an actually downloadable exact twin already served by the product, or derive and validate a separately justified compatibility mechanism before using610.88. Same branch alone is insufficient; no captured table or closed-binary reverse engineering was used.

Follow-up all-tag check: official git ls-remote lists610.43.02,610.43.03 and610.57.04. Their headers name610.46,610.62 and610.88 respectively; no published610 tag has matching Linux/Windows changelists. The unpinned610.43.03 was checked as required by the preferred-all-public-tags ruling. No exact public twin for610.88 was missed by only consulting the product matrix. This is a current public-source limitation, not proof no source can ever be released.
