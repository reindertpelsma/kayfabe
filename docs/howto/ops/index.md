# Run your own CI (operators)

These pages are for people who run kayfabe's continuous integration themselves: the Cloudflare Worker that rents GPU boxes, its budgets, and how to read what it reports. If you only want to install and use kayfabe, you do not need any of them: start with [Install](../installing.md).

What exists today is the operator's own guidance, written for the project's own deployment. A from-scratch deployment guide for someone else, an inventory of secret names and a script that creates the storage buckets are not written yet. The Worker's source and its own README are in `ci/vast-broker/` in the repository; `DEPLOY.md` beside it records what is and is not deployed.

| Page | What it answers |
|---|---|
| [Downloading pinned CI data](ci-downloads.md) | How the checked downloads of the CI work |
| [Plan work for a box or fleet](kfci-plan.md) | Turning a run catalog and box snapshots into a plan |
| [Default Vast check selection](vast-check-selection.md) | What the check runs by default |
| [Vast check capacity and spending settings](vast-budget-settings.md) | The limits on rentals and spending |
| [Provider faults and one replacement](vast-provider-retry.md) | What happens when a rented box is faulty |
| [When a GPU check reports a provider fault](vast-provider-faults.md) | How to read such a report |
| [Diagnose a Vast rerun authorization refusal](vast-rerun-auth.md) | Why a rerun is refused |
| [The Windows stage of the gpu-vast check](gpu-vast-windows.md) | The Windows part of the master gate |
| [Windows application gate policy](windows-app-gate-policy.md) | How the Windows application stage grades results |
| [What a failed GPU run leaves behind](gpu-vast-evidence.md) | The evidence bundle of a failed run |
| [Read a reused GPU gate](gpu-gate-reuse.md) | When the master gate reuses an earlier pass |
| [Read the Site commit status](site-status.md) | The site build's status check |

These pages keep the project's evidence labels (CODE READ, NOT DEPLOYED and so on) because their readers are the people who deploy and audit the CI.
