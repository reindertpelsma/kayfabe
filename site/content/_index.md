+++
title = "kayfabe"
description = "Share one NVIDIA GPU between a Linux desktop and Windows or Linux virtual machines."

# Source fields render beside their claims as HTML comments.
[extra]
page_title = "kayfabe: one GPU for your desktop and your VMs"
tagline = "Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it."
tagline_src = "README.md opening promise"
lede = "Run a Windows app or a Linux CUDA job without giving up your desktop."
lede_src = "README.md opening paragraph and What runs today"

[extra.screenshot]
src = "README.md screenshot; traces/20261021_131847.jpg (downscaled copy)"
alt = "Windows 11 and a Linux desktop using one RTX 4070, on two monitors."
caption = "Windows 11 on the right. Linux on the left. One machine, one RTX 4070."

[extra.uses]
heading = "What it is for"
src = "README.md What runs today; docs/howto/technical-faq.md test suite"

[[extra.uses.items]]
title = "A Windows app on Linux"
text = "Keep your Linux desktop while a Windows VM runs the app."
src = "README.md Windows applications and shared-desktop screenshot"

[[extra.uses.items]]
title = "CUDA in its own VM"
text = "Give a Linux job its own kernel and driver version."
src = "docs/howto/requirements.md Guest; README.md Linux guest"

[[extra.uses.items]]
title = "A fresh VM for each test"
text = "Boot it, use the GPU, then throw it away."
src = "docs/howto/technical-faq.md What do the gates and 30 arms mean"

[extra.different]
heading = "What is different"
text = "The host keeps the card. The VM uses NVIDIA's own driver, with no driver patch or vGPU licence server."
src = "README.md comparison table and Nothing is patched"

[extra.where]
heading = "Where it stands"
src = "README.md status and What runs today; docs/howto/index.md"

[[extra.where.items]]
text = "Linux desktops and CUDA jobs run. Windows 11 runs real 3D apps."
src = "README.md Windows and Linux results; docs/howto/requirements.md GPU coverage"

[[extra.where.items]]
text = "Windows is not stable. Sleep, wake and video playback still have problems."
src = "README.md Not finished; docs/howto/windows-guest.md"

[[extra.where.items]]
text = "Research project. There is no installer yet. Build from source."
src = "README.md Trying it; docs/howto/building.md"

[[extra.where.items]]
text = "Not security audited. Use one VM per card for now."
src = "README.md status; docs/howto/security.md"

[extra.start]
heading = "Try it"
text = "For developers who can build Rust and QEMU. Start with the requirements and build guide."
src = "README.md Trying it; docs/howto/building.md; docs/howto/installing.md"
+++
