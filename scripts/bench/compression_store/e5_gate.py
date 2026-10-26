"""Conservative interpreter for E5(a,b) JSONL. It cannot certify E5(c,d)."""
import argparse
import json
from pathlib import Path


def evaluate(rows):
    reasons = []
    starts = [r for r in rows if r.get('event') == 'start']
    if len(starts) != 1 or starts[0].get('uid', 0) == 0:
        reasons.append('missing or privileged probe identity')
    returned = [r for r in rows if r.get('event') == 'allocation_params'
                and r.get('phase') == 'returned']
    allocation = returned[0] if len(returned) == 1 else {}
    if allocation.get('compr', 0) not in (1, 3) or allocation.get('format') not in (8, 9):
        reasons.append('RM did not confirm a compression-capable generic allocation')
    coverage = [r for r in rows if r.get('event') == 'coverage']
    if (len(coverage) != 1 or coverage[0].get('format') != 1
            or not 0 < coverage[0].get('line_min', 0)
            <= coverage[0].get('line_max', -1) < 0xffffffff):
        reasons.append('missing or invalid comptag coverage')
    for page in (65536, 4096):
        for kind in (8, 0):
            maps = [r for r in rows if r.get('event') == 'map'
                    and (r.get('requested_page'), r.get('requested_kind')) == (page, kind)]
            if (len(maps) != 1 or maps[0].get('rc') != 0
                    or maps[0].get('status') != 0):
                reasons.append(f'map not successful: page={page} kind={kind}')
            ptes = [r for r in rows if r.get('event') == 'pte'
                    and (r.get('requested_page'), r.get('requested_kind')) == (page, kind)
                    and r.get('page') == page and r.get('kind') == kind
                    and r.get('flags', 0) & 1]
            if not ptes:
                reasons.append(f'valid matching PTE absent: page={page} kind={kind}')
    for row in rows:
        if row.get('event') in ('alloc', 'control', 'map', 'free'):
            if row.get('rc') != 0 or row.get('status') != 0:
                reasons.append(f"RM or syscall failure: {row.get('event')}/{row.get('label')}")
    finishes = [r for r in rows if r.get('event') == 'finish']
    if len(finishes) != 1 or finishes[0].get('failures') != 0:
        reasons.append('probe missing clean finish')
    return dict(ab_verified=not reasons,
                zbc_preference_retained=allocation.get('zbc') == 1,
                zbc_suppression_verified=False,
                fresh_compbits_verified=False,
                task2_eligible=False,
                reasons=reasons,
                limit='Allocation/PTE readback does not establish E5(c,d) or GPU bandwidth.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('jsonl', type=Path)
    args = parser.parse_args()
    rows = [json.loads(line) for line in args.jsonl.read_text().splitlines() if line.strip()]
    result = evaluate(rows)
    print(json.dumps(result, indent=2))
    return 0 if result['ab_verified'] else 2


if __name__ == '__main__':
    raise SystemExit(main())
