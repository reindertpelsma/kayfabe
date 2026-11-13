# kfml credits proposal

**STATUS: PROPOSAL, 2026-11-11.** Proposed lines for `docs/howto/credits.md`; that file is not edited by
the kfml PR. Licences below were checked against each repository's licence on 2026-11-11 and are
the author's reading, not legal advice.

| project | licence | URL | commit | how kfml uses it |
|---|---|---|---|---|
| nanoGPT (Andrej Karpathy) | MIT | https://github.com/karpathy/nanoGPT | 3adf61e154c3fe3fca428ad6bc3818b27a3b8291 | `model.py` vendored verbatim in `scripts/kfml/vendor/nanogpt/` |
| Triton tutorials | MIT | https://github.com/triton-lang/triton | d654e0f2d91f07496454e0fcbec2a9b97df37d47 (tag v3.3.1) | kernels written in the style of the tutorials; no code copied |
| char-rnn tinyshakespeare dataset | text public domain, file provenance unverified | https://github.com/karpathy/char-rnn | fetched from `master` raw URL, pinned by sha256 | dataset, fetched at run time, not redistributed |
| CIFAR-10 (Krizhevsky, 2009) | no licence stated | https://www.cs.toronto.edu/~kriz/cifar.html | pinned by sha256 | dataset, fetched at run time, not redistributed |
| PyTorch, torchvision, Triton, numpy, ninja | BSD-3-Clause, MIT, BSD-3-Clause, BSD-3-Clause/other, Apache-2.0 | PyPI and https://download.pytorch.org | exact versions in `requirements.lock.txt` | installed into the venv per box, not redistributed by us |

Proposed credits line: "kfml workloads use code from nanoGPT (MIT, Andrej Karpathy) and follow the
Triton tutorials (MIT); datasets: tiny Shakespeare (char-rnn) and CIFAR-10 (Alex Krizhevsky)."
