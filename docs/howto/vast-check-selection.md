# Default Vast check selection

**STATUS: LIVE source guidance, updated 2026-11-09.** The middle application policy
requires the matching Worker bundle and prepared media; its rollout is not performed here.

Since 2026-11-05 (branch `claude/worker-tier-20261105`, once deployed) the check of a **pull request
to master** picks its die by family instead: each run takes the family after the last one that ran
(Turing, Ampere, Ada, Blackwell), using that family's first die with an eligible offer, and falls
through to the next family when the family has none. The status shows the selection reason `ROTATE`; the Worker log
line `selection ... family=<f>` (and `fallback_from=<f>` after a fall-through) names the family. A
**push to master** keeps the cheapest-die rule below.

CODE READ from source at rev 9a995ad97 plus the selection change in this branch: an ordinary
`check` request considers every configured `CHECK_DIES` class that has eligible offers. It
keeps the configured country preference and effective-price ordering within each class,
then chooses the class whose selected offer has the lowest effective hourly price. Equal
prices retain configured die order. `DEFAULT_ORDER` remains the compatible protocol label
for this default policy. An explicit ordered die preference or sweep rotation keeps its
existing order.

Every verification, quality, country deny, machine exclusion and spending check still
applies. Availability lists eligible per-die prices; the runner prints the actual rental's
machine, host, instance, GPU and offer hourly price. The offer price excludes the broker's
transfer allowance; admission uses the full reservation estimate.

CODE READ (2026-11-08): the source default for `DENIED_COUNTRIES` is
`CU IR KP SY CN HK MO RU BY`. Offers matching the active deny list are rejected as
`COUNTRY` before price or country preference. The override accepts unique uppercase
two-letter codes.


## Applications in the middle check

With the matching Worker bundle installed, the 2026-11-09 middle-tier policy adds real
applications to the PR-to-master check: all 63 entries of the Linux middle gate list, then
the Windows gate, then the 10 Windows `mid` apps. `mid` includes all eight `small` apps.
Push/small keeps today's Linux/Windows check without the application stages. A release's
large-tier contract adds SPECviewperf and a real LLM workload; that orchestration remains
planned, and the unimplemented LLM stage is refused.

The middle check keeps the 300-minute box limit. It reserves 205 minutes for Linux apps,
Windows gate and Windows apps together, within the 290-minute soft window. If earlier work
leaves too little time, the next stage does not start and the check reports a TIME retry.
Apps are not silently removed to fit. Missing application media reports MEDIA retry; missing
app output cannot pass; a guest failure fails the check. Manual and unsupported apps receive
no passing credit. See [budget settings](vast-budget-settings.md) and
[CI downloads](ci-downloads.md).

The owner can set Worker var `MIDDLE_APPS_ENABLED=false` to stop requesting both application
stages for new runs without changing source. This is an owner control, not a PR input. The
initial policy change needs the new Worker bundle and its matching BASE runner/pin; changing
the var does not cancel an already admitted run. Deployment and first combined hardware
qualification were not performed for this source change.
