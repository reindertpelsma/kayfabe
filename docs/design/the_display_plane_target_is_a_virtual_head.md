# The Mode-2 display target is a virtual head, NOT displayless

**STATUS: LIVE, 2026-09-18 (w760r). Written to correct a chain of my own errors, and to
supersede a stale claim in the ARCHIVE that I propagated into this tree's reasoning.**

## ⊘⊘⊘ The stale claim, and where it is NOT

`/workspace/nvidia-gpu-passthrough/docs/design/display_plane_scoping.md` — **the nvkvm archive,
not this repo and not the shipped product** — says:

> every DRM-backend compositor (weston/mutter/sway) **hangs forever** in `libnvidia-egl-gbm`'s
> `gbm_surface`→scanout path … ⇒ the headless-compositor + capture route is the viable one

★ **The SHIPPED tree makes no such claim.** `grep -rniE "mutter.*hang|sway.*hang|hangs forever"`
over all of `/workspace/nvkvm-pv/docs` returns **nothing**. Its weston entry is weston-specific
and carries a weston stack trace; the generalisation to mutter and sway exists only in the
archive.

⚠ **How this got into the reasoning** (worth recording, because the mechanism is generic): a
`cd /workspace/kf-master && … display_plane_scoping.md` failed with *No such file* — this repo
does not have that doc — and the command was re-run WITHOUT the `cd`, so it resolved against the
session's default cwd, which is the archive. The text was then quoted as current guidance and a
correction was offered "in `display_plane_scoping.md`", a file kayfabe does not contain.
⇒ **A path that resolves is not a path in the tree you meant.** When a repo-relative read fails
once, the retry must re-assert the repo, not drop the `cd`.

## ★★★ What the SHIPPED product actually does — and it is the owner's model exactly

Owner, 2026-09-18: *"real KMS planes flow from guest to host, not displayless, the broker
proved. Full GNOME desktops and plasma started, in wayland no separate config was needed."*

`nvkvm-pv/docs/internal/known-limitations.md`:

> `nvkvm` presents a virtual KMS head to the guest (`src/guest/nvkvm_kms.c`), and **a compositor
> can drive it**: one connector of type `DRM_MODE_CONNECTOR_VIRTUAL`, one CRTC, one primary
> plane, a fixed 1920x1080@60 mode. … the virtual CRTC **accepts atomic commits / page-flips and
> completes their flip events**, but performs no real scanout (there is no physical connector).

⇒ *"There is no scanout path — intrinsic"* means **no physical connector**, NOT *no plane flow*.
`modetest` enumerates `Virtual-1 connected, 1920x1080, 23 modes`.

`nvkvm-pv/docs/internal/broker-design.md`, the hardware log (RTX 3090, 580.105.08):

> **A guest-shaped dma-buf reaches the screen on both backends.** The test buffer … comes back
> with modifier `0x0300000000606014` — one of the two block-linear modifiers
> `src/guest/nvkvm_kms.c` records off real guest bos — so it is **representative of what a guest
> actually flips**, not a linear stand-in.

Verified under **KDE Plasma** (X11/NVIDIA DDX on a forced virtual head) and **sway 1.7**, with
**GNOME Shell 50.1 / Mutter on Wayland** in the later log. X was partly software-rendered
because the **DDX** was broken — a legacy X-only issue, bypassable, and not a KMS-plane problem.

⇒ The flow is: guest compositor → guest virtual KMS head → real `PAGE_FLIP` of a real guest bo →
flip event completes → dma-buf → broker → host compositor. **Real KMS planes, guest to host.**

## ⇒ THE TARGET, corrected

⊘ **`NVA083_GRID_DISPLAYLESS` is the HEADLESS-COMPUTE answer and is the WRONG target if we want
desktops.** Its defining property is `.coreChannelDma = { }` — zero channels, no head. Aiming
there forfeits the working desktop path rather than reaching it. It remains the right answer for
a headless-compute product, and the two are a **product decision**, not a discovery.

★ The Mode-2 goal is a **virtual head that a STOCK NVKMS accepts**, whose flips we export as
dma-bufs to the broker that already exists.

| half | state |
|---|---|
| host present — dma-buf → host EGL → window, cross-isolate dma-buf brokering | ✔ **built and hardware-verified** (`nvkvm_present_egl.c`, 569 LOC, + broker) |
| guest virtual head | ⊘ **new work.** `nvkvm_kms.c` (313 LOC) is a *guest kernel module*; Mode 2 runs a **stock** driver, so it cannot be shipped. The guest's real NVKMS must be satisfied instead. |

⇒ The cost is **"satisfy a known consumer whose source we vendor"**, not "emulate EVO". EVO's
method stream is mechanical; NVKMS acceptance is the work.

⚠ And the security argument does not bite here: *"the display engine is host-global singular
hardware with no per-context containment"* rules out **forwarding** a display channel. It says
nothing against **emulating** one — which is what keeps kayfabe unprivileged, since we never
touch the real display engine at all.
