# Summary

[Introduction](index.md)

# Part 1: Use kayfabe

- [Requirements](requirements.md)
- [Install](installing.md)
  - [Building from source](building.md)
- [Run a Linux guest](linux-guest.md)
  - [Attach to a VM you already have](attach-to-existing-vm.md)
- [Run a Windows guest](windows-guest.md)
- [Display and the broker](display.md)
- [Command-line reference](cli-reference.md)
- [Troubleshooting](troubleshooting.md)
- [Report a problem](report-a-problem.md)
- [Security model](security.md)
- [Technical FAQ](technical-faq.md)
- [Goals and plans](goals.md)
- [Credits and notices](credits.md)

# Part 2: Test it yourself

- [What you can test](test-yourself.md)
  - [Run everything in one go](full-standalone-run.md)
  - [Run the open FFmpeg and DCGM rows](open-linux-rows.md)
  - [Run the Vulkan conformance subset](vulkan-cts.md)
  - [Run the open ML rows (vLLM, TensorRT, JAX)](open-ml-rows.md)
  - [Run the open Windows rows](windows-open-rows.md)
  - [Rebuild the Linux apps image](linux-apps-image.md)
  - [The Windows test harness](windows-harness.md)
  - [Bring your own Windows images](run-windows-images.md)
  - [Proprietary test images](proprietary-test-images.md)
  - [Recreate the proprietary Windows image](recreate-the-proprietary-image.md)

# Part 3: Run your own CI (operators)

- [Overview](ops/index.md)
  - [Downloading pinned CI data](ops/ci-downloads.md)
  - [Plan work for a box or fleet](ops/kfci-plan.md)
  - [Default Vast check selection](ops/vast-check-selection.md)
  - [Vast check capacity and spending settings](ops/vast-budget-settings.md)
  - [Provider faults and one replacement](ops/vast-provider-retry.md)
  - [When a GPU check reports a provider fault](ops/vast-provider-faults.md)
  - [Diagnose a Vast rerun authorization refusal](ops/vast-rerun-auth.md)
  - [The Windows stage of the gpu-vast check](ops/gpu-vast-windows.md)
  - [Windows application gate policy](ops/windows-app-gate-policy.md)
  - [What a failed GPU run leaves behind](ops/gpu-vast-evidence.md)
  - [Read a reused GPU gate](ops/gpu-gate-reuse.md)
  - [Read the Site commit status](ops/site-status.md)
