# Third-party notices

**STATUS: LIVE, 2026-11-20 (section 4 rewritten for task I7).** What kayfabe uses from other projects, under which licence, and the
notices those licences ask us to carry. kayfabe's own licence is in [`LICENSE`](LICENSE)
(`Apache-2.0 OR GPL-2.0-or-later`, with the exceptions listed there). Trademarks and
vendor-software notes are in [`docs/NOTICES.md`](docs/NOTICES.md).

kayfabe ships no NVIDIA or Microsoft binaries. The tables below are what is built into, derived
into, or only referenced by this tree.

## 1. Data and layouts derived from NVIDIA sources (MIT)

### 1.1 NVIDIA open-gpu-kernel-modules

- Upstream: <https://github.com/NVIDIA/open-gpu-kernel-modules>, licence MIT (the dual MIT/GPL-2.0
  statement in its `COPYING` applies to a Linux kernel module linked from it; kayfabe is not one).
- Pinned as submodules `third_party/ogkm` and `third_party/ogkm-580` (reference sources: not built,
  not linked, not copied into the tree). The pinned revisions are listed in
  [`third_party/README.md`](third_party/README.md).
- Struct layouts, class ids, control ids, register offsets and bit fields are generated from its
  headers by the tools below, and the generated files carry the notice that follows:
  - `crates/kf-abi/gen/` writes `crates/kf-abi/src/generated/{classes,ctrl,nvos,rpc,vbios,mod}.rs`
  - `tools/drivermatrix/emit_rust.py` writes `crates/kf-abi/src/generated/matrix.rs`
  - `tools/api_boundaries/ingress_policy.py` writes `crates/kf-abi/src/ingress_generated.rs` and
    `tools/api_boundaries/ingress_facts.tsv` (each control's export flags and each class's allocation
    privilege, per driver tag; the guest-ingress allowlist, section 4)
  - `tools/derive_hwref.sh` writes `crates/kf-chip/data/hwref-*.tsv`
- `tools/uvm_efs/patch/` is a patch to the nvidia-uvm sources and is MIT like them.
  `tools/host_patches/h5_userd_dma/include/nvkms-kapi.h` is an NVIDIA header kept with its own
  notice.

The notice text of the upstream `COPYING` (it is the same text that the generated files carry):

    Copyright (c) 2021 NVIDIA CORPORATION & AFFILIATES. All rights reserved.

    Permission is hereby granted, free of charge, to any person obtaining a
    copy of this software and associated documentation files (the "Software"),
    to deal in the Software without restriction, including without limitation
    the rights to use, copy, modify, merge, publish, distribute, sublicense,
    and/or sell copies of the Software, and to permit persons to whom the
    Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
    THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
    FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
    DEALINGS IN THE SOFTWARE.

### 1.2 NVIDIA open-gpu-doc

- Upstream: <https://github.com/NVIDIA/open-gpu-doc>, licence MIT.
- The video-engine class method headers (`classes/video/`) are read by
  `tools/derive_video_methods.py`, which writes `crates/kf-chan/src/video_generated.rs`; that file
  carries the notice that follows.

    Copyright (c) 2019, NVIDIA CORPORATION. All rights reserved.

    Permission is hereby granted, free of charge, to any person obtaining a
    copy of this software and associated documentation files (the "Software"),
    to deal in the Software without restriction, including without limitation
    the rights to use, copy, modify, merge, publish, distribute, sublicense,
    and/or sell copies of the Software, and to permit persons to whom the
    Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL
    THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
    FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
    DEALINGS IN THE SOFTWARE.

## 2. QEMU (GPL-2.0)

The `kf3-gpu` device in `qemu/hw/misc/kf3/` is built inside a QEMU source tree. QEMU as a whole is
released under the GNU General Public License, version 2 (<https://www.qemu.org/>, `LICENSE` in the
QEMU tree). The QEMU tree itself is not part of this repository. Every kayfabe file in that
directory names its terms in an SPDX line: `Apache-2.0 OR GPL-2.0-or-later`. The files that are
Apache-2.0 only are listed in `LICENSE`, exception 2; read it before building or redistributing a
QEMU binary that links them.

## 3. Rust crates

The shipped crates (`crates/kf-*`, including the `kf-qemu` static library that the QEMU device
links) have one external crate in their dependency closure (`cargo tree -e normal,build`,
2026-10-23):

| crate | version | licence |
|---|---|---|
| [libc](https://github.com/rust-lang/libc) | 0.2.189 | MIT OR Apache-2.0 |

`firmware/kf-gop` (the UEFI display driver) depends only on workspace crates. The generator
`crates/kf-abi/gen/` has no dependencies.

`Cargo.lock` also lists crates used only by tests and developer tools (for example `loom`,
`trybuild`, `tracing`, `serde`, `toml`, `regex-automata`). None is linked into a shipped
artifact. Their licences, as declared in their manifests, are MIT, Apache-2.0, Unlicense and
Unicode-3.0 (or an `OR` combination of them). Run `cargo metadata` for the complete, current list.

## 4. gVisor (Apache-2.0): design reference, and files in the frozen tree

**The guest-ingress allowlist.** The design of a default-deny allowlist of RM controls and allocation
classes at the guest ingress follows the `nvproxy` package of gVisor (<https://github.com/google/gvisor>,
<https://gvisor.dev/docs/user_guide/gpu/>). kayfabe's allowlist (`crates/kf-abi/src/capability.rs`, the
generated `crates/kf-abi/src/ingress_generated.rs`) is generated from NVIDIA's open-gpu-kernel-modules
(MIT, section 1) and kayfabe's own measurements and judgements by `tools/api_boundaries/ingress_policy.py`
(task I7, 2026-11-20); no nvproxy code or tables are included in it.

**Files in the frozen tree.** Some files outside the QEMU device still derive from nvproxy; `LICENSE`,
exception 2, lists them (the frozen `crates/kayfabe-abi` crate and `archive/`), and they are available
under the Apache License 2.0 only. gVisor is pinned as the reference submodule `third_party/gvisor`. Its
notice:

> Copyright The gVisor Authors.
>
> Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in
> compliance with the License. You may obtain a copy of the License at
> <http://www.apache.org/licenses/LICENSE-2.0> (also `LICENSE-APACHE` in this repository). Unless required
> by applicable law or agreed to in writing, software distributed under the License is distributed on an
> "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License
> for the specific language governing permissions and limitations under the License.

## 5. Reference sources that are only cited

`third_party/` also pins the Linux kernel (`third_party/linux`, read for the nouveau and nova
drivers) as a reference. It is not built, linked or copied into kayfabe. Each file there is under
the licence in its own header. `third_party/` is not initialised by a normal clone.

## 6. Other licences inside the tree

- `broker/` (the host display broker, imported from the author's `nvkvm-pv` repository,
  see `broker/IMPORTED.md`): files are marked `GPL-2.0 OR Apache-2.0`, where the GPL option is
  version 2 only. The repository default is `GPL-2.0-or-later` (`LICENSE`). Either option may be
  chosen. The broker is built against the system's X11, XCB, Wayland and GBM libraries through
  `pkg-config`; those libraries are not distributed here.
- `archive/` (frozen): see `LICENSE`, exception 3, and `archive/nvkvm/CREDITS`.
- Any file that carries its own copyright or licence notice is under that notice (`LICENSE`,
  exception 3).
- Traces, logs and captures under `traces/`, `docs/` and `archive/` contain data produced by
  third-party software and hardware; see `LICENSE`, exception 4.
