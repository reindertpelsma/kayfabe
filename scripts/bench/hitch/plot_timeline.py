#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""plot_timeline.py — time-aligned view of one Heaven pass: guest (Heaven/ETW), kayfabe (kf3 per-second counters), broker.

  plot_timeline.py --dispts H.dispts.txt --f9 H.p1.f9_epoch --guestlog hv_h_p1.log --out DIR/NAME [--broker-lines FILE]
                   [--title T] [--span 0 262] [--etw-offset 7200] [--no-align]

Inputs (all produced by scripts/bench/candidate_hw + the hitch harness; nothing is read from the network):
  --dispts      epoch-stamped `kf3: family=` / `kf3: display fps[` lines of the QEMU log (chainh.sh tails them)
  --f9          the host epoch at which F9 started the Heaven benchmark (t = 0 on every axis)
  --guestlog    the heaven_etw job log: `FR <start FILETIME> <dur 100ns>` lines (DXGI Present, Heaven pid)
  --broker-lines  optional: the broker's per-second stats lines (`nvkvm-broker: stats t=<CLOCK_MONOTONIC ms> ...`), with
                  --broker-epoch-offset (host epoch minus CLOCK_MONOTONIC seconds) to put them on the same axis

Outputs NAME_A.png (three stacked panels sharing x: Heaven fps / kayfabe counters / broker counters) and NAME_B.png
(CUMULATIVE frames at each stage on one axis + the gap between consecutive stages; vertical lines: observed broker request
timeouts and FREE-fallback events). Flat stretch = stall, steep catch-up after it = burst repayment, a gap that never
closes = lost frames. Measurement only.
"""
import argparse, re, sys, bisect

TOK = re.compile(r'([A-Za-z_][\w>\-]*)\[|\]|([A-Za-z_][\w>\-]*)=(-?(?:0x[0-9a-fA-F]+|[0-9]+(?:\.[0-9]+)?))')


def parse_kv(s):
    d, stack = {}, []
    for m in TOK.finditer(s):
        if m.group(1):
            stack.append(m.group(1))
        elif m.group(0) == ']':
            if stack:
                stack.pop()
        else:
            k = '.'.join(stack + [m.group(2)]) if stack else m.group(2)
            v = m.group(3)
            d.setdefault(k, float(int(v, 16)) if v.startswith('0x') else float(v))
    return d


def load_family(path):
    rows = []
    for ln in open(path, errors='replace'):
        p = ln.split(' ', 1)
        if len(p) < 2 or 'family=' not in p[1]:
            continue
        try:
            t = float(p[0])
        except ValueError:
            continue
        d = parse_kv(p[1])
        m = re.search(r'src=(\w+)', p[1])
        d['_src_broker'] = 1.0 if (m and m.group(1) == 'broker') else 0.0
        rows.append((t, d))
    return rows


def load_frames(path, off):
    out = []
    for ln in open(path, errors='replace'):
        if ln.startswith('FR '):
            p = ln.split()
            try:
                out.append(int(p[1]) / 1e7 - 11644473600 + off)
            except (IndexError, ValueError):
                pass
    return out


def etw_rate(frames, lo, hi, bin_s):
    n = int((hi - lo) / bin_s)
    c = [0] * n
    for t in frames:
        k = int((t - lo) / bin_s)
        if 0 <= k < n:
            c[k] += 1
    return [lo + (k + 0.5) * bin_s for k in range(n)], [x / bin_s for x in c]


def align(frames, fam, f9, off0):
    """Offset (s) added to the guest clock that best matches ETW fps to kayfabe's tear per window (the guest clock is not synced)."""
    best = None
    wins = []
    prev = None
    for t, d in fam:
        if prev is not None and 0 <= t - f9 <= 260:
            wins.append((prev - f9, t - f9, d['fps.h0.tear']))
        prev = t
    for k in range(-60, 61):
        o = off0 + k * 0.05
        fr = sorted(x - f9 + (o - off0) for x in frames)
        err = 0.0
        for a, b, tear in wins:
            n = bisect.bisect_left(fr, b) - bisect.bisect_left(fr, a)
            err += (n / max(b - a, 1e-6) - tear) ** 2
        if best is None or err < best[0]:
            best = (err, o)
    return best[1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--dispts', required=True)
    ap.add_argument('--f9', required=True)
    ap.add_argument('--guestlog', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--title', default='')
    ap.add_argument('--span', nargs=2, type=float, default=[0, 262])
    ap.add_argument('--etw-offset', type=float, default=7200.0)
    ap.add_argument('--no-align', action='store_true')
    ap.add_argument('--broker-lines')
    ap.add_argument('--broker-epoch-offset', type=float, default=0.0)
    a = ap.parse_args()
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt

    f9 = float(open(a.f9).read().strip())
    lo, hi = a.span
    fam = load_family(a.dispts)
    fr = load_frames(a.guestlog, a.etw_offset)
    off = a.etw_offset if a.no_align else align(fr, fam, f9, a.etw_offset)
    fr_rel = sorted(x - f9 + (off - a.etw_offset) for x in fr)
    # kayfabe: per-window rates and cumulative counters
    T, tear, kms, copies, tick, checks = [], [], [], [], [], []
    dgrant, dmbox, dcc, dcd, dskip, dwant, dframes, dnoch = ([] for _ in range(8))
    src, silent_at, timeout_at = [], [], []
    prev = None
    cum_tear = [0.0]
    cum_copies = [0.0]
    cum_frames = [0.0]
    cum_t = [lo]
    for t, d in fam:
        r = t - f9
        if 'fps.h0.tear' not in d or 'tick.want' not in d:
            continue  # a status line the relay's lock was busy for: no tick[...] / fps[...] fragment; skip, never plot a zero
        if prev is not None and lo <= r <= hi:
            dt = t - prev[0]
            p = prev[1]
            T.append(r)
            tear.append(d['fps.h0.tear'])
            kms.append(d['fps.h0.kms'])
            copies.append(d['fps.copies'])
            tick.append(d['fps.h0.tick'])
            checks.append(d['fps.checks'])
            dd = lambda k: (d.get(k, 0) - p.get(k, 0)) / dt
            dgrant.append(dd('fps.lock.granted'))
            dmbox.append(dd('fps.lock.mailbox_coalesced'))
            dcc.append(dd('fps.lock.credits_coalesced'))
            dcd.append(dd('fps.lock.credits_deferred'))
            dskip.append(dd('fps.skipped'))
            dwant.append(dd('tick.want'))
            dframes.append(dd('tick.frames_on_credit'))
            dnoch.append(dd('tick.no_change'))
            src.append(d['_src_broker'])
            cum_t.append(r)
            cum_tear.append(cum_tear[-1] + d['fps.h0.tear'] * dt)
            cum_copies.append(cum_copies[-1] + d['fps.copies'] * dt)
            cum_frames.append(cum_frames[-1] + dd('tick.frames_on_credit') * dt)
            if d.get('tick.silence_fallbacks', 0) > p.get('tick.silence_fallbacks', 0):
                silent_at.append(r)
            if d.get('tick.credits_coalesced', 0) > p.get('tick.credits_coalesced', 0):
                timeout_at.append(r)
        prev = (t, d)
    ex, ey = etw_rate(fr_rel, lo, hi, 1.0)
    broker = None
    if a.broker_lines:
        broker = []
        for ln in open(a.broker_lines, errors='replace'):
            m = re.search(r'stats t=(\d+) (.*)', ln)
            if m:
                d = parse_kv(m.group(2))
                d['t'] = int(m.group(1)) / 1000.0 + a.broker_epoch_offset - f9
                broker.append(d)

    def vlines(ax):
        for x in timeout_at:
            ax.axvline(x, color='tab:red', alpha=.35, lw=.8)
        for x in silent_at:
            ax.axvline(x, color='tab:purple', alpha=.6, lw=1.2, ls='--')

    # ---- graph A
    fig, ax = plt.subplots(3, 1, sharex=True, figsize=(15, 10))
    ax[0].plot(ex, ey, color='tab:green', label='Heaven: DXGI Present/s (guest ETW, 1 s bins)')
    ax[0].plot(T, tear, color='k', lw=1, label='kayfabe: tear (immediate flips)/s')
    ax[0].axhline(60, color='grey', ls=':', lw=.8)
    ax[0].set_ylabel('frames / s')
    ax[0].legend(loc='lower right', fontsize=8)
    ax[0].set_title((a.title + '  ' if a.title else '') + 'guest/ETW clock offset %+.2f s vs kayfabe (auto-aligned on tear)' % (off - a.etw_offset))
    ax[1].plot(T, tick, label='ticks/s', color='tab:blue')
    ax[1].plot(T, kms, label='kms (flips)/s', color='tab:orange', lw=.8)
    ax[1].plot(T, copies, label='copies/s', color='tab:red')
    ax[1].plot(T, dgrant, label='WANT grants/s (lock.granted)', color='tab:cyan', lw=.8)
    ax[1].plot(T, dframes, label='frames on credit/s', color='tab:olive', lw=.8, ls='--')
    ax[1].plot(T, checks, label='checks/s', color='tab:pink')
    ax[1].plot(T, dmbox, label='mailbox_coalesced/s', color='tab:brown', lw=.8)
    ax[1].plot(T, dcc, label='credits_coalesced/s', color='tab:gray', lw=.8)
    ax[1].plot(T, dcd, label='credits_deferred/s', color='black', lw=.6, ls=':')
    ax[1].plot(T, dskip, label='skipped/s', color='tab:purple', lw=.8)
    ax[1].plot(T, [60 * s for s in src], color='tab:green', alpha=.15, lw=6, label='tick source = broker (band)')
    ax[1].set_ylabel('kayfabe / s')
    ax[1].legend(loc='center left', bbox_to_anchor=(1.0, 0.5), fontsize=7)
    if broker:
        for k, lab in (('feedbacks', 'feedbacks'), ('ticks', 'ticks sent'), ('want_ticks', 'WANT ticks'), ('commit', 'COMMIT answered'),
                       ('no_change', 'NO_CHANGE answered'), ('timeouts', 'request timeouts'), ('late', 'late'), ('unsolicited', 'unsolicited'),
                       ('presented', 'presented frames')):
            ax[2].plot([b['t'] for b in broker], [b.get(k, 0) for b in broker], label=lab, lw=1)
        ax[2].legend(loc='center left', bbox_to_anchor=(1.0, 0.5), fontsize=7)
    else:
        ax[2].text(0.5, 0.5, 'no broker per-second line for this arm (the broker printed totals at exit only)', ha='center', va='center',
                   transform=ax[2].transAxes, color='grey')
    ax[2].set_ylabel('broker / s')
    ax[2].set_xlabel('seconds after F9 (benchmark start)')
    for x in ax:
        vlines(x)
        x.grid(alpha=.2)
    fig.tight_layout()
    fig.savefig(a.out + '_A.png', dpi=110)
    plt.close(fig)

    # ---- graph B: cumulative
    g_t = [x for x in fr_rel if lo <= x <= hi]
    g_cum_t = g_t
    g_cum = list(range(1, len(g_t) + 1))

    def interp(xs, ys, x):
        i = bisect.bisect_right(xs, x)
        if i <= 0:
            return 0.0
        if i >= len(xs):
            return ys[-1]
        x0, x1, y0, y1 = xs[i - 1], xs[i], ys[i - 1], ys[i]
        return y0 + (y1 - y0) * (x - x0) / max(x1 - x0, 1e-9)

    grid = [lo + 0.5 * k for k in range(int((hi - lo) / 0.5) + 1)]
    g0 = bisect.bisect_right(g_t, cum_t[1] - (cum_t[1] - lo) * 0 - 0.0) if len(cum_t) > 1 else 0
    first_win_start = lo  # the first kayfabe window starts at the first family line after lo; counters integrate from there
    guest_c = [bisect.bisect_right(g_t, x) - bisect.bisect_right(g_t, cum_t[1] - 2.0) for x in grid]
    flips_c = [interp(cum_t, cum_tear, x) - cum_tear[0] for x in grid]
    copies_c = [interp(cum_t, cum_copies, x) - cum_copies[0] for x in grid]
    sent_c = [interp(cum_t, cum_frames, x) - cum_frames[0] for x in grid]
    fig, ax = plt.subplots(2, 1, sharex=True, figsize=(15, 9), gridspec_kw={'height_ratios': [3, 2]})
    ax[0].plot(grid, guest_c, label='guest presents (ETW)', color='tab:green')
    ax[0].plot(grid, flips_c, label='kayfabe flips latched (sum of tear/s)', color='k', lw=1)
    ax[0].plot(grid, copies_c, label='kayfabe copies (sum of copies/s)', color='tab:red')
    ax[0].plot(grid, sent_c, label='frames sent on a request (relay)', color='tab:olive', ls='--')
    if broker:
        bt = [b['t'] for b in broker]
        for k, lab in (('presented', 'broker presented (cum)'), ('commit', 'broker COMMIT (cum)')):
            c, acc = [], 0.0
            for b in broker:
                acc += b.get(k, 0)
                c.append(acc)
            ax[0].plot(bt, c, label=lab, lw=1)
    ax[0].set_ylabel('cumulative frames since F9')
    ax[0].legend(fontsize=8, loc='upper left')
    ax[0].set_title((a.title + '  ' if a.title else '') + 'cumulative frames per stage; flat = stall, steep after = burst repayment, never closing = lost')
    ax[1].plot(grid, [g - f for g, f in zip(guest_c, flips_c)], label='guest presents - flips', color='tab:green')
    ax[1].plot(grid, [f - c for f, c in zip(flips_c, copies_c)], label='flips - copies (skipped/merged)', color='k')
    ax[1].plot(grid, [c - s for c, s in zip(copies_c, sent_c)], label='copies - frames sent (held/blocked)', color='tab:red')
    ax[1].set_ylabel('gap (frames)')
    ax[1].set_xlabel('seconds after F9')
    ax[1].legend(fontsize=8, loc='upper left')
    for x in ax:
        vlines(x)
        x.grid(alpha=.2)
    fig.text(0.01, 0.005, 'red lines: relay saw an unanswered request replaced (broker request timeout); purple dashed: 50 ms FREE fallback', fontsize=8)
    fig.tight_layout()
    fig.savefig(a.out + '_B.png', dpi=110)
    plt.close(fig)
    print('wrote', a.out + '_A.png', a.out + '_B.png', 'offset', round(off - a.etw_offset, 2), 'timeouts', len(timeout_at), 'fallbacks', len(silent_at))


if __name__ == '__main__':
    main()
