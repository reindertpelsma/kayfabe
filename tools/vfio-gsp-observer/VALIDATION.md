# Local validation

**STATUS: RESEARCH, 2026-10-05.** No borrowed-PC mutation or GPU run was performed
for this implementation. Prefix coverage remains unverified on hardware.

Host toolchain: Ubuntu GCC 15.2.0, Clang 21.1.8, Python 3.14.4. QEMU upstream
`v10.2.4` / `3e0bcba1ca7d6607ca49a988d165f052a3a53323`, with this directory's
patch and copied sources, compiled and linked as `qemu-system-x86_64`.

Local compile-only configuration (the eventual runtime build must retain the
matched baseline's options and use a separate output binary):

```sh
./configure --target-list=x86_64-softmmu --disable-docs --disable-gtk \
  --disable-sdl --disable-vnc --disable-curses --disable-tools --disable-user \
  --disable-tcg --disable-werror --disable-slirp --disable-capstone --disable-debug-info
ninja -C build -j6 qemu-system-x86_64 libqom.a
```

The final local binary SHA256 is
`00bbca387232a8c9ffad5d44b24d0f17b67e1467e3c173ae28c8a0e335b742cb`.
No executable is committed. The SHA records this local compile result only.

Passed:

- Portable bootstrap/queue core under both GCC and Clang ASan/UBSan, strict
  warnings: valid command prefix before status initialization, later replies,
  no duplicate history, observed sequence loss, unstable copies, changed page
  mapping, reused allocation generation, malformed geometry/pointers/non-RAM.
- Pinned OGKM 580.65.06 TU102 and GA102 compiler layouts, including LibOS,
  message queue arguments, mailbox/doorbell and Falcon IRQ registers.
- Pinned patch installer: dry check, altered source refusal, existing
  destination refusal, successful application and unrelated-source preservation.
- Production asynchronous writer: 6,500 8192-byte payloads exceed the 32 MiB
  FIFO capacity over time, preserving order and both hashes through ring wrap,
  final drain and strict decode; source-byte and record caps, saturated FIFO,
  `/dev/full`, corrupt metadata and truncated output refusals.
- Regression for stop acquired after an earlier empty-queue observation.
- Real QEMU MemoryRegion/RAMBlock predicates reject RAM-device, ROM, non-RAM,
  protected and guest-memfd states, and allow ordinary RAM. This unit test does
  not boot a VM or exercise actual PCI address-space translation.
- Decode preserves generations and refuses cross-generation or ambiguous
  query request/reply pairing.

Independent review identified and resolved the RAM-device classification and
final writer drain bugs before deployment. No claim is made about GPU coverage,
Windows timing, or complete initialization. Clean QEMU exit/error receipts and
uninstrumented health controls remain required for future runtime evidence.
