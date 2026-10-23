#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Round4 rental limits and a pre-armed, exact-owned-ID destroy guard."""
import datetime
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time

DENIED_COUNTRIES = {'CU', 'IR', 'KP', 'SY', 'CUBA', 'IRAN', 'NORTH KOREA', 'SYRIA'}


def validate_offer(offer):
    rate = float(offer['dph_total'])
    country = str(offer.get('geolocation') or '').strip()
    tokens = {part.strip().upper() for part in country.split(',')}
    if not country or tokens & DENIED_COUNTRIES:
        raise ValueError('country unavailable or owner-denied')
    if not math.isfinite(rate) or not 0 < rate <= 2.50:
        raise ValueError('owner price guard')
    if not offer.get('vms_enabled') or offer.get('num_gpus') != 1:
        raise ValueError('one KVM-capable GPU required')
    if float(offer.get('disk_space', 0)) < 180:
        raise ValueError('180GB disk required')


def reservation(offer, *, spent, credit, now, minutes=170):
    validate_offer(offer)
    values = [spent, credit, now, minutes]
    if not all(math.isfinite(value) for value in values) or spent < 0:
        raise ValueError('invalid budget inputs')
    if not 0 < minutes <= 170:
        raise ValueError('retain stricter170-minute auto-destroy')
    down, up = (float(offer.get(key, math.inf))
                for key in ('inet_down_cost', 'inet_up_cost'))
    if not all(math.isfinite(x) and x >= 0 for x in (down, up)):
        raise ValueError('missing transfer price')
    transfer = max(.25, 80 * down + 2 * up + .10)
    rate = float(offer['dph_total'])
    guarded_rate = max(rate * 1.15, rate + .10)
    reserved = guarded_rate * minutes / 60 + transfer
    if spent + reserved >= 2.50 or credit - reserved < 5:
        raise ValueError('round stop2.50 or credit reserve5')
    return {'deadline_epoch': now + minutes * 60, 'reserved_usd': reserved,
            'transfer_reserved_usd': transfer, 'rate_guard': guarded_rate,
            'round_stop_usd': 2.50, 'round_cap_usd': 3.00, 'minutes': minutes}


def armed_create(arm, create, register):
    if arm() is not True:
        raise ValueError('destroy guard not armed before create')
    reply = create()
    iid = reply.get('new_contract')
    if reply.get('success') is not True or type(iid) is not int or iid <= 0:
        raise ValueError('create did not return an exact owned id')
    register(iid)
    return iid


def owned_id(plan, intent):
    if plan.get('intent') != intent:
        raise ValueError('foreign intent')
    iid = plan.get('id')
    if iid is not None and (type(iid) is not int or iid <= 0):
        raise ValueError('invalid exact own id')
    return iid


def save(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    with temporary.open('w') as out:
        json.dump(value, out, indent=2)
        out.write('\n')
        out.flush()
        os.fsync(out.fileno())
    temporary.replace(path)


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def watch(path):
    plan = json.loads(path.read_text())
    intent, deadline = plan['intent'], float(plan['deadline_epoch'])
    remaining = deadline - time.time()
    if not 0 < remaining <= 170 * 60:
        raise ValueError('prearm must precede allocation and fixed expiry')
    if owned_id(plan, intent) is not None:
        raise ValueError('guard must start BEFORE create')
    monotonic_end = time.monotonic() + remaining
    save(path.with_suffix('.ready.json'),
         {'intent': intent, 'pid': os.getpid(), 'deadline_epoch': deadline,
          'armed_utc': utc(), 'state': 'ARMED_BEFORE_CREATE'})
    target = None
    while True:
        current = json.loads(path.read_text())
        if current['deadline_epoch'] != deadline:
            raise ValueError('destroy deadline changed')
        registered = owned_id(current, intent)
        if target is not None and target != registered:
            raise ValueError('registered own id changed')
        target = registered
        if time.monotonic() >= monotonic_end:
            save(path.with_suffix('.expired.json'), {'intent': intent, 'utc': utc()})
            if target is not None:
                break
            # A bounded in-flight create may return its id after the deadline.
            if time.monotonic() >= monotonic_end + 180:
                return
        time.sleep(1)
    while True:
        try:
            subprocess.run(['vastai', 'destroy', 'instance', str(target), '-y'],
                           capture_output=True, timeout=45)
            reply = subprocess.run(['vastai', 'show', 'instances', '--raw'],
                                   capture_output=True, text=True, timeout=45, check=True)
            rows = json.loads(reply.stdout)
            if not isinstance(rows, list):
                raise ValueError('instance list is not an array')
            if not any(int(row['id']) == target for row in rows):
                save(path.with_suffix('.absent.json'),
                     {'intent': intent, 'id': target, 'api_absence_utc': utc()})
                return
        except (subprocess.SubprocessError, ValueError, KeyError, TypeError) as error:
            save(path.with_suffix('.retry.json'),
                 {'intent': intent, 'id': target, 'error_type': type(error).__name__,
                  'utc': utc(), 'state': 'OWNED_ID_REAPER_RETRY'})
        time.sleep(1)



if __name__ == '__main__':
    if len(sys.argv) != 3 or sys.argv[1] != 'watch':
        raise SystemExit('usage: lease_guard.py watch INTENT.json')
    watch(Path(sys.argv[2]))
