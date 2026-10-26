+++
title = "kayfabe"
+++

**Share an NVIDIA GPU with virtual machines that run the stock NVIDIA driver, while the host keeps using it.**

Kayfabe is a QEMU device that shows a guest an emulated NVIDIA GPU. The guest loads NVIDIA's real,
unmodified driver (Windows or Linux); kayfabe reads what that driver asks for and has the real
host GPU do the work, from an **unprivileged** host process that talks to the host driver as an
ordinary client. Written in Rust.

**Status: research.** It runs real applications, but there is no installer, no stable interface and
no security audit yet. The [README](https://github.com/reindertpelsma/kayfabe#readme) carries the
current status.

- Nothing is patched: no host or guest driver or kernel is modified.
- The guest is treated as hostile: every host action is authored by kayfabe from a few unprivileged
  verbs, never forwarded from guest bytes.

## Get it

- Source and releases: [github.com/reindertpelsma/kayfabe](https://github.com/reindertpelsma/kayfabe)
  ([releases](https://github.com/reindertpelsma/kayfabe/releases)). Build from source; this site is
  documentation only and never the place to get code you have to trust.
- [Documentation](/docs/)
