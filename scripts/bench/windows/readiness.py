#!/usr/bin/env python3
"""Shared bench readiness policy. OWNER-APPROVED 2026-10-23; no driver/TDR policy."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


# One policy, reused unchanged from SW2 commit4ede56ac7; this file is its S4 CLI adapter.
_SHARED_PATH = Path(__file__).resolve().parents[1] / 'blackwell/readiness.py'
_SHARED_SPEC = importlib.util.spec_from_file_location('shared_box_readiness', _SHARED_PATH)
SHARED = importlib.util.module_from_spec(_SHARED_SPEC)
_SHARED_SPEC.loader.exec_module(SHARED)


class BudgetExpired(RuntimeError):
    pass


class ReadinessTimeout(RuntimeError):
    pass


class CommandFailed(RuntimeError):
    pass


def positive(value):
    if isinstance(value, bool) or not math.isfinite(value) or value <= 0:
        raise ValueError('positive finite timing required')
    return value


def make_profile(boot_seconds, query_seconds, *, box_id, source):
    positive(boot_seconds); positive(query_seconds)
    if type(box_id) is not int or box_id <= 0 or len(source) != 40 or any(c not in '0123456789abcdef' for c in source):
        raise ValueError('owned box and source identity required')
    # Express baseline boot/query slowdown as the shared policy's half-second probe units.
    probe_seconds = max(boot_seconds / 60, query_seconds / 2) * .5
    scale = SHARED.policy(probe_seconds)['factor']
    return dict(schema=1, box_id=box_id, source=source, boot_seconds=boot_seconds,
                query_seconds=query_seconds, scale=scale)


def write_profile(path, profile):
    with Path(path).open('x') as f:
        json.dump(profile, f, indent=2); f.write('\n')


def read_profile(path):
    p = json.loads(Path(path).read_text())
    expected = make_profile(p['boot_seconds'], p['query_seconds'], box_id=p['box_id'], source=p['source'])
    if any(p[k] != v for k, v in expected.items()):
        raise ValueError('calibration profile changed')
    return p


def step_seconds(nominal, profile, ceiling, remaining):
    positive(nominal); positive(ceiling)
    if not math.isfinite(remaining) or remaining <= 0:
        raise BudgetExpired('stage budget exhausted')
    scale = profile['scale']
    if type(scale) is not int or not 4 <= scale <= 10:
        raise ValueError('scale outside owner range')
    return SHARED.Budget(ceiling, remaining, clock=lambda: 0).timeout(nominal * scale)


def stage_seconds(nominal, profile, ceiling, lease_remaining, reserve):
    if not math.isfinite(reserve) or reserve < 0:
        raise ValueError('invalid export reserve')
    return step_seconds(nominal, profile, ceiling, lease_remaining - reserve)


def transient(result):
    if result.returncode == 124:
        return True
    error = result.stderr.lower()
    return result.returncode == 255 and any(s in error for s in (
        'banner exchange', 'connection timed out', 'connection refused', 'connection reset by peer'))


def run_query(operation, *, profile, nominal, ceiling, remaining, marker=None, record=lambda _: None):
    """Adapt authored read-only transport to SW2's single first-query retry."""
    initial = remaining()
    step_seconds(nominal, profile, ceiling, initial)
    budget = SHARED.Budget(2 * ceiling, initial)
    attempts = 0
    def fetch(core_limit):
        nonlocal attempts
        attempts += 1
        limit = step_seconds(nominal, profile, min(ceiling, core_limit), remaining())
        try:
            result = operation(limit)
        except subprocess.TimeoutExpired:
            result = subprocess.CompletedProcess([], 124, '', '')
        left = remaining()
        record(dict(attempt=attempts, timeout_seconds=limit, returncode=result.returncode,
                    remaining_seconds=left))
        if left <= 0:
            raise BudgetExpired('result arrived after the stage deadline')
        if transient(result):
            raise subprocess.TimeoutExpired('authored-read-only-query', limit)
        if result.returncode:
            raise CommandFailed('read-only command failed: ' + str(result.returncode))
        if marker is not None and marker not in [x.strip() for x in result.stdout.splitlines()]:
            raise CommandFailed('actual readiness marker absent')
        return result
    try:
        return SHARED.first_query(fetch, budget, nominal * profile['scale'])
    except subprocess.TimeoutExpired as error:
        raise ReadinessTimeout('two transient readiness failures; no state evidence') from error
    except TimeoutError as error:
        raise BudgetExpired('shared readiness budget exhausted') from error


def run_once(operation, *, profile, nominal, ceiling, remaining, marker=None, record=lambda _: None):
    """Authored side effects have one attempt; partial/timed-out output is not evidence."""
    limit = step_seconds(nominal, profile, ceiling, remaining())
    try:
        result = operation(limit)
    except subprocess.TimeoutExpired:
        result = subprocess.CompletedProcess([], 124, '', '')
    left = remaining()
    record(dict(attempt=1, timeout_seconds=limit, returncode=result.returncode, remaining_seconds=left))
    if left <= 0:
        raise BudgetExpired('single-attempt step exhausted its stage')
    if result.returncode == 124:
        raise ReadinessTimeout('single-attempt command timed out; not retried')
    if result.returncode:
        raise CommandFailed('single-attempt command failed: ' + str(result.returncode))
    if marker is not None and marker not in [x.strip() for x in result.stdout.splitlines()]:
        raise CommandFailed('actual command marker absent')
    return result


def digest(profile):
    return hashlib.sha256(json.dumps(profile, sort_keys=True).encode()).hexdigest()


def lease_remaining(lease, profile):
    if lease['id'] != profile['box_id'] or lease['source'] != profile['source']:
        raise ValueError('calibration belongs to a different box/source')
    created = datetime.datetime.fromisoformat(lease['created_utc'])
    end = created + datetime.timedelta(minutes=lease['auto_destroy_minutes'])
    return (end - datetime.datetime.now(datetime.timezone.utc)).total_seconds()


def bounded_command(command, limit):
    """GNU timeout terminates its owned process group; output is projected separately."""
    env = dict(os.environ, SSH_TIMEOUT=str(limit),
               WINVM_SSH_CONNECT_TIMEOUT=str(max(1, math.ceil(min(180, limit)))))
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        p = subprocess.run(['timeout', '-k', '5s', str(limit) + 's', *command],
                           stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, env=env)
        stdout.seek(0); stderr.seek(0)
        out, err = stdout.read(65537), stderr.read(65537)
        if len(out) > 65536 or len(err) > 65536:
            raise CommandFailed('readiness output byte bound')
        return subprocess.CompletedProcess(command, p.returncode,
                                           out.decode(errors='replace'), err.decode(errors='replace'))


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest='mode', required=True)
    c = sub.add_parser('calibrate'); c.add_argument('--out', required=True)
    c.add_argument('--lease', required=True); c.add_argument('--boot-seconds', type=float, required=True)
    c.add_argument('command', nargs=argparse.REMAINDER)
    s = sub.add_parser('stage'); s.add_argument('--out', required=True); s.add_argument('--lease', required=True)
    s.add_argument('--profile', required=True); s.add_argument('--nominal', type=float, required=True)
    s.add_argument('--ceiling', type=float, required=True); s.add_argument('--reserve', type=float, default=1200)
    for mode in ('wait', 'query', 'once'):
        q = sub.add_parser(mode); q.add_argument('--profile', required=True); q.add_argument('--stage', required=True)
        q.add_argument('--nominal', type=float, required=True); q.add_argument('--ceiling', type=float, required=True)
        q.add_argument('--marker'); q.add_argument('--receipt', required=True)
        if mode == 'wait':
            q.add_argument('--probe-nominal', type=float, default=20)
            q.add_argument('--probe-ceiling', type=float, default=180)
        q.add_argument('command', nargs=argparse.REMAINDER)
    a = p.parse_args()
    if a.mode == 'stage':
        profile = read_profile(a.profile); lease = json.loads(Path(a.lease).read_text())
        seconds = stage_seconds(a.nominal, profile, a.ceiling, lease_remaining(lease, profile), a.reserve)
        write_profile(a.out, dict(profile_sha256=digest(profile), box_id=profile['box_id'],
                                 source=profile['source'], created_utc=utc(), seconds=seconds,
                                 end_monotonic=time.monotonic() + seconds, export_reserve_seconds=a.reserve))
        print('READINESS_STAGE seconds=' + str(seconds)); return 0
    command = a.command[1:] if a.command[:1] == ['--'] else a.command
    if not command:
        p.error('authored read-only command required')
    if a.mode == 'calibrate':
        if Path(a.out).exists():
            raise FileExistsError('box calibration already recorded')
        lease = json.loads(Path(a.lease).read_text())
        initial = make_profile(600, 20, box_id=lease['id'], source=lease['source'])
        end = time.monotonic() + stage_seconds(18, initial, 180, lease_remaining(lease, initial), 1200)
        samples = []
        def operation(limit):
            start = time.monotonic(); r = bounded_command(command, limit)
            samples.append(max(time.monotonic() - start, limit if transient(r) else .001))
            return r
        run_query(operation, profile=initial, nominal=12, ceiling=120,
                  remaining=lambda: end - time.monotonic(), marker='KF_READINESS_SAMPLE')
        profile = make_profile(a.boot_seconds, max(samples), box_id=lease['id'], source=lease['source'])
        profile.update(created_utc=utc(), query_attempts=len(samples))
        write_profile(a.out, profile)
        print('BOX_READINESS_RECORDED scale=' + str(profile['scale'])); return 0
    profile = read_profile(a.profile); stage = json.loads(Path(a.stage).read_text())
    if stage['profile_sha256'] != digest(profile) or stage['box_id'] != profile['box_id']:
        raise ValueError('stage belongs to another calibration')
    end = stage['end_monotonic']
    receipt = dict(started_utc=utc(), box_id=profile['box_id'], source=profile['source'],
                   scale=profile['scale'], profile_sha256=digest(profile), mode=a.mode, attempts=[])
    def record(row):
        if len(receipt['attempts']) < 1024:
            receipt['attempts'].append(row)
    try:
        if a.mode == 'wait':
            end = min(end, time.monotonic() + step_seconds(a.nominal, profile, a.ceiling,
                                                         end - time.monotonic()))
            while True:
                try:
                    result = run_query(lambda limit: bounded_command(command, limit), profile=profile,
                                       nominal=a.probe_nominal, ceiling=a.probe_ceiling,
                                       remaining=lambda: end - time.monotonic(), marker=a.marker, record=record)
                    break
                except ReadinessTimeout:
                    if end - time.monotonic() <= 0:
                        raise BudgetExpired('readiness stage exhausted')
                    time.sleep(min(5, end - time.monotonic()))
        else:
            runner = run_once if a.mode == 'once' else run_query
            result = runner(lambda limit: bounded_command(command, limit), profile=profile,
                               nominal=a.nominal, ceiling=a.ceiling, remaining=lambda: end - time.monotonic(),
                               marker=a.marker, record=record)
        receipt['result'] = 'ACTUAL_REPLY'; print(result.stdout, end=''); return 0
    except (BudgetExpired, ReadinessTimeout) as e:
        receipt['result'] = 'READINESS_INCONCLUSIVE'; receipt['reason'] = str(e)
        print('READINESS_INCONCLUSIVE ' + str(e), file=sys.stderr); return 75
    finally:
        receipt['finished_utc'] = utc()
        Path(a.receipt).write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    sys.exit(main())
