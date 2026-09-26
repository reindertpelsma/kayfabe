#!/usr/bin/env python3
# torch_train.py — a short PyTorch TRAINING workload for the refusal audit: ResNet-18 on random
# data, fp32 then AMP, forward+backward+SGD, with synchronizes long enough to put the waiter to
# sleep. Emits CHECK lines and TORCH_TRAIN_DONE. Seeded; the loss is checked finite and falling.
import time, torch, torch.nn as nn, torchvision.models as M
assert torch.cuda.is_available()
torch.manual_seed(0)
net = M.resnet18(weights=None, num_classes=10).cuda()
opt = torch.optim.SGD(net.parameters(), lr=0.01, momentum=0.9)
x = torch.randn(64, 3, 224, 224, device="cuda"); y = torch.randint(0, 10, (64,), device="cuda")
def step(amp):
    with torch.autocast("cuda", enabled=amp):
        loss = nn.functional.cross_entropy(net(x), y)
    opt.zero_grad(set_to_none=True); loss.backward(); opt.step()
    return loss
for amp in (False, True):
    t0 = time.time(); losses = []
    for i in range(12):
        losses.append(step(amp).item())
    torch.cuda.synchronize()
    ok = all(l == l and l < 1e4 for l in losses) and losses[-1] < losses[0]
    print(f"CHECK train_{'amp' if amp else 'fp32'} {'ok' if ok else 'FAIL'} first={losses[0]:.3f} last={losses[-1]:.3f} secs={time.time()-t0:.1f}", flush=True)
# one long synchronize (> 1 s of queued work) — the blocked-waiter shape
t0 = time.time()
for _ in range(40):
    step(True)
torch.cuda.synchronize()
print(f"CHECK long_sync ok secs={time.time()-t0:.1f}", flush=True)
print("TORCH_TRAIN_DONE", flush=True)
