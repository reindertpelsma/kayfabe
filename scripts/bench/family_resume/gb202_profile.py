"""Explicit GB202-only preparation profile; no provider calls or implicit admission."""

import datetime as dt
from decimal import Decimal
import hashlib
import json
import math
from pathlib import Path

PROFILE_ID = 'GB202_5090_90MIN'
SOURCE = '7dcedefea093de078188efd5f712561bde746362'
MAX_RATE = 1.50
STOP_USD = 3.20
CAP_USD = 3.25
MAX_SECONDS = 5400
TARGET = (58156107, 155138, 79466)
DENIED = {'CU', 'IR', 'KP', 'SY', 'CN', 'HK', 'MO'}


def number(value):
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError('finite numeric profile input required')
    return value


def check_offer(offer):
    identity = tuple(offer.get(key) for key in ('id', 'machine_id', 'host_id'))
    if identity != TARGET or offer.get('host_id') == 70221 or offer.get('machine_id') == 30498:
        raise ValueError('exact GB202 candidate and unchanged blacklist required')
    if not (
        offer.get('verification') == 'verified'
        and offer.get('vms_enabled') is True
        and offer.get('is_vm_deverified') is False
        and type(offer.get('num_gpus')) is int
        and offer['num_gpus'] == 1
    ):
        raise ValueError('verified single VM-capable GPU required')
    country = offer.get('geolocation', '').rsplit(',', 1)[-1].strip()
    if len(country) != 2 or country in DENIED:
        raise ValueError('approved country required')
    for key, floor in [
        ('cpu_cores_effective', 8),
        ('cpu_ram', 16384),
        ('disk_space', 180),
        ('reliability', 0.95),
        ('inet_down', 200),
        ('gpu_ram', 10240),
    ]:
        if number(offer.get(key)) < floor:
            raise ValueError('profile resource floor: ' + key)
    if not 0 < number(offer.get('dph_total')) <= MAX_RATE:
        raise ValueError('GB202 profile quote ceiling exceeded')
    for key in ('inet_down_cost', 'inet_up_cost'):
        if number(offer.get(key)) < 0:
            raise ValueError('negative transfer rate')


def make_plan(offer, credit, anchor):
    check_offer(offer)
    credit = number(credit)
    if anchor.utcoffset() is None or credit <= 0:
        raise ValueError('aware fixed anchor and positive credit required')
    anchor = anchor.astimezone(dt.timezone.utc)
    quoted = offer['dph_total']
    rate = max(quoted * 1.15, quoted + 0.10)
    transfer = 12 * offer['inet_down_cost'] + offer['inet_up_cost']
    cost_seconds = int((STOP_USD - 0.50 - transfer) * 3600 / rate) - 60
    if cost_seconds < MAX_SECONDS:
        raise ValueError('complete proposed5400s allocation cannot fit profile stop')
    reservation = rate * MAX_SECONDS / 3600 + 0.50 + transfer
    if not reservation < STOP_USD < CAP_USD or credit - reservation < 8:
        raise ValueError('profile reservation or reserve refused')
    return dict(
        credit_before=credit,
        transfer_usd=transfer,
        nonruntime_margin_usd=0.50,
        armed_utc=anchor.isoformat(),
        deadline_utc=(anchor + dt.timedelta(seconds=MAX_SECONDS)).isoformat(),
        seconds=MAX_SECONDS,
        quoted_rate=quoted,
        rate_bound=rate,
        reservation_usd=reservation,
        credit_after_reservation=credit - reservation,
        stop_usd=STOP_USD,
        cap_usd=CAP_USD,
        reserve_usd=8.0,
        leases_authorized=1,
        capture_lead_seconds=600,
        work_lead_seconds=660,
        framebuffer_mib=8192,
        profile_id=PROFILE_ID,
        target_die='GB202',
        source=SOURCE,
    )


def check_plan(plan, offer, now):
    anchor = dt.datetime.fromisoformat(plan['armed_utc'])
    expected = make_plan(offer, plan['credit_before'], anchor)
    if set(plan) != set(expected):
        raise ValueError('fixed plan fields differ from reviewed profile')
    for key, value in expected.items():
        actual = plan[key]
        if type(value) is float:
            equal = abs(number(actual) - value) <= 1e-9
        else:
            equal = type(actual) is type(value) and actual == value
        if not equal:
            raise ValueError('fixed plan mismatch: ' + key)
    if now.utcoffset() is None or now < anchor:
        raise ValueError('invalid admission clock')
    if (dt.datetime.fromisoformat(plan['deadline_utc']) - now).total_seconds() <= 660:
        raise ValueError('fixed work cutoff expired; never reset the plan')
    return plan


def check_budget(budget, plan):
    def amount(key):
        value = Decimal(str(budget[key]))
        if not value.is_finite() or value < 0:
            raise ValueError('invalid full budget amount: ' + key)
        return value

    for key, floor in [
        ('prior_sw5_stage_hold_usd', '2.794777622'),
        ('old_closed_ci_hold_usd', '1'),
        ('retry_closed_ci_hold_usd', '2'),
    ]:
        if amount(key) < Decimal(floor):
            raise ValueError('old conservative hold omitted: ' + key)
    if (
        budget.get('complete_account_inventory') is not True
        or type(budget.get('account_instances_observed')) is not int
        or budget['account_instances_observed'] != 0
    ):
        raise ValueError('complete empty admission inventory required')
    credit = amount('credit_usd')
    if abs(credit - Decimal(str(plan['credit_before']))) > Decimal('.000000001'):
        raise ValueError('credit projection differs from fixed plan')
    total = sum(
        amount(key)
        for key in (
            'prior_sw5_stage_hold_usd',
            'today_billing_all_charges_usd',
            'old_closed_ci_hold_usd',
            'retry_closed_ci_hold_usd',
            'other_liabilities_usd',
        )
    ) + Decimal(str(plan['reservation_usd']))
    if total > 12 or credit - total < 8:
        raise ValueError('shared cap or reserve refused; fresh root admission required')
    return total


def require_go(here, offer=None, plan=None):
    """Read exact future root proof; neither generate a plan nor persist an intent."""
    here = Path(here)
    root = here.parent
    review = json.loads((root / 'root_guard_go.json').read_text())
    manifest_path = root / 'review_manifest_gb202.json'
    plan_path = root / 'gb202_admitted_plan.json'
    budget_path = root / 'gb202_admitted_budget.json'

    def read_pinned(path, key):
        raw = path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != review.get(key):
            raise ValueError('root did not admit exact ' + key)
        return json.loads(raw)

    manifest = read_pinned(manifest_path, 'manifest_sha256')
    pinned_plan = read_pinned(plan_path, 'plan_sha256')
    budget = read_pinned(budget_path, 'budget_sha256')
    if not (
        review.get('decision') == 'GO'
        and review.get('profile_id') == PROFILE_ID
        and manifest.get('profile_id') == PROFILE_ID
        and bool(manifest.get('guard_head'))
        and review.get('guard_head') == manifest.get('guard_head')
        and manifest.get('product_source') == SOURCE
    ):
        raise ValueError('explicit root admission for this exact profile/source required')
    for name, digest in manifest['operator_sources'].items():
        if Path(name).name != name:
            raise ValueError('profile artifact name is not a basename')
        changed = hashlib.sha256((here / name).read_bytes()).hexdigest() != digest
        if changed:
            raise ValueError('profile artifact changed')
    for name, digest in manifest['runtime_source_digests'].items():
        if hashlib.sha256(Path(name).read_bytes()).hexdigest() != digest:
            raise ValueError('reviewed runtime dependency changed')
    selected = manifest['selected_offer']
    check_offer(selected)
    blacklist = json.loads((here / 'known_bad_machines.json').read_text())
    if any(
        row.get('machine_id') == selected['machine_id'] or row.get('host_id') == selected['host_id']
        for row in blacklist
    ):
        raise ValueError('full unchanged machine/host blacklist refuses this candidate')
    if review.get('selected_offer') != selected or (offer is not None and offer != selected):
        raise ValueError('selected offer differs from exact root admission')
    if offer is not None and plan != pinned_plan:
        raise ValueError('adapter did not use exact admitted fixed plan')
    check_plan(pinned_plan, selected, dt.datetime.now(dt.timezone.utc))
    check_budget(budget, pinned_plan)
    return dict(review, plan=pinned_plan, budget=budget, selected_offer=selected)
