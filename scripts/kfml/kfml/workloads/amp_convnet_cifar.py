"""amp_convnet_cifar: small timm-free ResNet-style net on a CIFAR-10 subset with AMP (bf16, or fp16 +
GradScaler on Turing), channels_last, cudnn.benchmark, a DataLoader with 2 worker processes and
pin_memory, plus an explicit CUDA IPC round trip through a spawned process."""
from __future__ import annotations

from ..spec import Spec

SPEC = Spec(
    name="amp_convnet_cifar", module="kfml.workloads.amp_convnet_cifar",
    description="ResNet-like CIFAR-10 subset, AMP + channels_last + workers + pin_memory + CUDA IPC",
    tier="mid", budget_s=1200, launch_bound=True, min_vram_gb=8.0, min_cc=(7, 5), min_torch="2.7",
    datasets=("cifar10_binary",),
    download_note="first run downloads several GB of torch/CUDA wheels (setup) plus the 163 MB CIFAR-10 archive")

N_TRAIN, N_TEST, EPOCHS, BATCH = 20000, 2000, 5, 128
ACC_FLOOR = 0.45  # chance is 0.10; this net reaches roughly 0.6+ in this schedule (UNVERIFIED on hardware)
AGREE_FLOOR = 0.93  # fp32 vs autocast predictions on the test subset
TOL_REFERENCE = 0.10


def _load_cifar(path, files, n_per_file=10000):
    import tarfile
    import numpy as np
    xs, ys = [], []
    want = {"cifar-10-batches-bin/" + f for f in files}
    with tarfile.open(path, "r:gz") as tf:
        for m in tf:  # only regular files whose names we asked for; nothing is extracted to disk
            if m.name in want and m.isfile():
                raw = np.frombuffer(tf.extractfile(m).read(), dtype=np.uint8).reshape(-1, 3073)
                ys.append(raw[:, 0].copy())
                xs.append(raw[:, 1:].reshape(-1, 3, 32, 32).copy())
    if sum(len(y) for y in ys) != n_per_file * len(files):
        raise RuntimeError("unexpected CIFAR-10 record count")
    return np.concatenate(xs), np.concatenate(ys)


class _Aug:
    """Dataset with deterministic per-(epoch, index) crop/flip augmentation."""

    def __init__(self, x, y, mean, std, train):
        self.x, self.y, self.mean, self.std, self.train, self.epoch = x, y, mean, std, train, 0

    def __len__(self):
        return len(self.x)

    def __getitem__(self, i):
        import numpy as np
        import torch
        im = self.x[i]
        if self.train:
            r = np.random.RandomState(self.epoch * 1_000_003 + int(i))
            pad = np.pad(im, ((0, 0), (4, 4), (4, 4)), mode="reflect")
            dx, dy = r.randint(0, 9, 2)
            im = pad[:, dy:dy + 32, dx:dx + 32]
            if r.rand() < 0.5:
                im = im[:, :, ::-1]
        t = torch.from_numpy(np.ascontiguousarray(im)).float().div_(255.0)
        t = (t - self.mean) / self.std
        return t, int(self.y[i])


def _net(torch):
    nn = torch.nn

    class Block(nn.Module):
        def __init__(self, cin, cout, stride):
            super().__init__()
            self.c1 = nn.Conv2d(cin, cout, 3, stride, 1, bias=False)
            self.b1 = nn.BatchNorm2d(cout)
            self.c2 = nn.Conv2d(cout, cout, 3, 1, 1, bias=False)
            self.b2 = nn.BatchNorm2d(cout)
            self.sc = None
            if stride != 1 or cin != cout:
                self.sc = nn.Sequential(nn.Conv2d(cin, cout, 1, stride, bias=False), nn.BatchNorm2d(cout))

        def forward(self, x):
            o = torch.relu(self.b1(self.c1(x)))
            o = self.b2(self.c2(o))
            return torch.relu(o + (x if self.sc is None else self.sc(x)))

    class Net(nn.Module):
        def __init__(self):
            super().__init__()
            self.stem = nn.Sequential(nn.Conv2d(3, 32, 3, 1, 1, bias=False), nn.BatchNorm2d(32), nn.ReLU())
            self.layers = nn.Sequential(Block(32, 32, 1), Block(32, 64, 2), Block(64, 128, 2), Block(128, 256, 2))
            self.head = nn.Linear(256, 10)

        def forward(self, x):
            x = self.layers(self.stem(x))
            return self.head(torch.flatten(torch.nn.functional.adaptive_avg_pool2d(x, 1), 1))

    return Net()


def _ipc_roundtrip(torch, C):
    import torch.multiprocessing as mp
    from . import _ipc_helper
    c = mp.get_context("spawn")
    t = torch.zeros(1024, device="cuda")
    done = c.Event()
    p = c.Process(target=_ipc_helper.bump, args=(t, done))
    p.start()
    ok = done.wait(120)
    p.join(30)
    torch.cuda.synchronize()
    C.check(ok and p.exitcode == 0, "CUDA IPC child failed (exit %s)" % p.exitcode)
    C.check(float(t.sum()) == 1024.0, "CUDA IPC write from another process not visible (sum=%s)" % float(t.sum()))


def run(ctx):
    import time
    import numpy as np
    import torch
    from .. import datasets
    from . import _common as C

    dev = ctx.device
    ds = datasets.fetch("cifar10_binary", ctx.data_dir)
    xtr, ytr = _load_cifar(ds["path"], ["data_batch_%d.bin" % i for i in range(1, 5)])
    xte, yte = _load_cifar(ds["path"], ["test_batch.bin"])
    rs = np.random.RandomState(ctx.seed)
    tr_idx = rs.permutation(len(xtr))[:N_TRAIN if not ctx.dev_cpu else 2000]
    te_idx = rs.permutation(len(xte))[:N_TEST if not ctx.dev_cpu else 500]
    xtr, ytr, xte, yte = xtr[tr_idx], ytr[tr_idx], xte[te_idx], yte[te_idx]
    mean = torch.tensor([0.4914, 0.4822, 0.4465]).view(3, 1, 1)
    std = torch.tensor([0.2470, 0.2435, 0.2616]).view(3, 1, 1)
    train_ds, test_ds = _Aug(xtr, ytr, mean, std, True), _Aug(xte, yte, mean, std, False)

    C.seed_all(torch, ctx.seed, dev)
    torch.backends.cudnn.benchmark = True
    torch.backends.cuda.matmul.allow_tf32 = False
    dtype = C.amp_dtype(torch, ctx.cc) if not ctx.dev_cpu else torch.bfloat16
    use_scaler = dtype == torch.float16 and dev == "cuda"
    model = _net(torch).to(dev)
    if dev == "cuda":
        model = model.to(memory_format=torch.channels_last)
    epochs = EPOCHS if not ctx.dev_cpu else 2
    steps_total = epochs * (len(train_ds) // BATCH)
    opt = torch.optim.SGD(model.parameters(), lr=0.05, momentum=0.9, weight_decay=5e-4, nesterov=True)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=0.1, total_steps=steps_total)
    scaler = torch.amp.GradScaler("cuda", enabled=use_scaler)
    gen = torch.Generator().manual_seed(ctx.seed)

    def loader(ds, shuffle, drop_last):
        return torch.utils.data.DataLoader(ds, batch_size=BATCH, shuffle=shuffle, drop_last=drop_last, num_workers=2,
                                           pin_memory=(dev == "cuda"), generator=gen if shuffle else None)

    def to_dev(x, y):
        x = x.to(dev, non_blocking=True)
        if dev == "cuda":
            x = x.contiguous(memory_format=torch.channels_last)
        return x, y.to(dev, non_blocking=True)

    epoch_loss, t0 = [], time.time()
    for ep in range(epochs):
        train_ds.epoch = ep
        model.train()
        tot, cnt = torch.zeros((), device=dev), 0
        for x, y in loader(train_ds, True, True):
            x, y = to_dev(x, y)
            with torch.autocast(device_type=dev, dtype=dtype):
                loss = torch.nn.functional.cross_entropy(model(x), y)
            scaler.scale(loss).backward()
            scaler.step(opt)
            scaler.update()
            opt.zero_grad(set_to_none=True)
            sched.step()
            tot += loss.detach().float()
            cnt += 1
            if ctx.expired():
                raise C.CheckFailed("soft wall-clock budget exhausted in epoch %d" % ep)
        epoch_loss.append(float(tot) / max(cnt, 1))
    train_s = time.time() - t0
    C.check(all(v == v and v < 1e6 for v in epoch_loss), "non-finite training loss: %s" % epoch_loss)
    C.check(epoch_loss[-1] < epoch_loss[0], "loss did not decrease: %s" % epoch_loss)

    model.eval()
    correct_amp = correct_fp32 = agree = total = 0
    with torch.no_grad():
        for x, y in loader(test_ds, False, False):
            x, y = to_dev(x, y)
            with torch.autocast(device_type=dev, dtype=dtype):
                pa = model(x).argmax(1)
            pf = model(x).argmax(1)
            correct_amp += int((pa == y).sum())
            correct_fp32 += int((pf == y).sum())
            agree += int((pa == pf).sum())
            total += len(y)
    acc, acc32, agr = correct_amp / total, correct_fp32 / total, agree / total
    if not ctx.dev_cpu:
        C.check(acc >= ACC_FLOOR, "AMP test accuracy %.3f below floor %.2f" % (acc, ACC_FLOOR))
        C.check(agr >= AGREE_FLOOR, "AMP vs fp32 prediction agreement %.3f < %.2f" % (agr, AGREE_FLOOR))
    ref_diff = None
    if ctx.reference and "acc" in ctx.reference:
        ref_diff = abs(ctx.reference["acc"] - acc)
        C.check(ref_diff <= TOL_REFERENCE, "accuracy differs from recorded native reference by %.3f" % ref_diff)
    if dev == "cuda":
        _ipc_roundtrip(torch, C)
    return {"metrics": {"amp_dtype": str(dtype), "grad_scaler": use_scaler, "epoch_loss": epoch_loss, "acc_amp": acc,
                        "acc_fp32": acc32, "pred_agreement": agr, "train_s": train_s, "reference_diff": ref_diff,
                        "cuda_ipc": dev == "cuda"},
            "dataset_sha256": {"cifar10_binary": ds["sha256"]}, "reference": {"acc": acc}}
