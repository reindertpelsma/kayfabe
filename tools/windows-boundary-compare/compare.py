#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline repeated boundary observations; no inferred identities or completeness."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import struct
import sys

MAX_INPUT = 64 * 1024 * 1024
MAX_EVENTS = 100000
MAX_TOTAL_EVENTS = 250000
MAX_RUNS = 32
IDENTITY_FIELDS = ('baseline_sha256', 'windows_driver', 'driver_hash', 'gpu', 'qemu_revision')
NATIVE_SCHEMA = 'kayfabe-gsp-observer/1'


class InvalidEvidence(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True)


def digest(blob):
    return hashlib.sha256(blob).hexdigest()


def read_bounded(path, limit=MAX_INPUT):
    with path.open('rb') as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, f'{path.name}: exceeds {limit} byte limit')
    return data


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f'duplicate JSON key: {key}')
        result[key] = value
    return result


def parse_json(data):
    try:
        return json.loads(data, object_pairs_hook=object_pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(InvalidEvidence('non-finite JSON number')))
    except (ValueError, UnicodeError, RecursionError) as error:
        raise InvalidEvidence(f'invalid JSON: {error}') from error


def text(value, name):
    require(isinstance(value, str) and 0 < len(value) <= 4096, f'{name}: nonempty bounded string required')
    return value


def u32(value, name):
    if isinstance(value, str):
        require(re.fullmatch(r'(?:0x[0-9a-fA-F]{1,8}|[0-9]{1,10})', value) is not None,
                f'{name}: invalid integer')
        value = int(value, 16 if value.startswith('0x') else 10)
    require(type(value) is int and 0 <= value <= 0xffffffff, f'{name}: expected uint32')
    return value


def u64(value, name):
    require(type(value) is int and 0 <= value <= 0xffffffffffffffff, f'{name}: expected uint64')
    return value


def hexadecimal(value):
    return None if value is None else f'0x{value:08x}'


def boolean(value, name):
    require(type(value) is bool, f'{name}: boolean required')
    return value


def event(fn, direction, selector=None, kind=None, status=None, inner=None):
    return dict(function=fn, direction=direction, selector_kind=kind,
                selector=hexadecimal(selector), status=hexadecimal(status),
                control_status=hexadecimal(inner))


def parse_kf(data):
    try:
        lines = data.decode('utf-8-sig').splitlines()
    except UnicodeError as error:
        raise InvalidEvidence('kf-rm log is not UTF-8') from error
    events, ignored = [], 0
    for line in lines:
        require(len(line) <= 262144, 'kf-rm line exceeds limit')
        if 'kf-rm: rpc-trace' not in line:
            ignored += 1
            continue
        # Logger is a text protocol; numeric handles and seq deliberately excluded.
        match = re.search(r'kf-rm: rpc-trace fn=(\d+) (\w+(?:\(\d+\))?) seq=(\d+) (.*?)result=(none|0x[0-9a-fA-F]+)\s*$', line)
        require(match is not None, 'malformed kf-rm rpc-trace line')
        fn, _, seq, detail, result = match.groups()
        fn = u32(fn, 'fn')
        u32(seq, 'seq')
        kind = 'control' if fn == 76 else 'class' if fn == 103 else None
        selector = None
        if kind:
            field = 'cmd' if kind == 'control' else 'class'
            matches = re.findall(rf'\b{field}=(undecodable|0x[0-9a-fA-F]+)(?=\s|$)', detail)
            require(len(matches) == 1, f'kf-rm: missing/duplicate {field}')
            if matches[0] != 'undecodable':
                selector = u32(matches[0], field)
        events.append(event(fn, 'completed', selector, kind,
                            None if result == 'none' else u32(result, 'result')))
        require(len(events) <= MAX_EVENTS, 'too many RPC events')
    return events, dict(ignored_lines=ignored, result_none=sum(e['status'] is None for e in events))


def parse_native(data, fmt):
    if fmt == 'kgwt-decoded-json':
        doc = parse_json(data)
        require(isinstance(doc, dict), 'native JSON must be an object')
        rows = doc.get('observations', doc.get('records'))
        if isinstance(doc.get('records'), int) and isinstance(rows, list):
            require(doc['records'] == len(rows), 'native record count mismatch')
    else:
        lines = data.splitlines()
        require(lines and all(len(line) <= 262144 for line in lines), 'empty/oversized native JSONL line')
        doc = parse_json(lines[0])
        require(isinstance(doc, dict) and doc.get('kind') == 'header', 'decoded JSONL requires header')
        rows = [parse_json(line) for line in lines[1:]]
        require(doc.get('records') == len(rows), 'decoded JSONL count mismatch/truncated file')
    require(doc.get('schema') == NATIVE_SCHEMA and doc.get('complete') is False,
            'native input must be decoded kayfabe-gsp-observer/1 with complete=false; run decode.py first')
    require(isinstance(rows, list) and len(rows) <= MAX_EVENTS, 'missing/oversized native observations')
    events, gaps, unknown, incomplete = [], 0, 0, 0
    for row in rows:
        require(isinstance(row, dict), 'native record must be object')
        direction = row.get('direction')
        require(direction in ('request', 'reply'), 'native direction must be request/reply')
        fn = u32(row.get('rpc_function'), 'rpc_function')
        status = u32(row.get('rpc_status'), 'rpc_status')
        gap = u32(row.get('missing_before'), 'missing_before')
        prefix = boolean(row.get('prefix_unknown'), 'prefix_unknown')
        selector = inner = None
        kind = 'control' if fn == 76 else 'class' if fn == 103 else None
        observed_complete = None
        control = row.get('control')
        if control is not None:
            require(fn == 76 and isinstance(control, dict), 'control on non-control record')
            if 'command' in control and 'cmd' in control:
                require(u32(control['command'], 'command') == u32(control['cmd'], 'cmd'), 'conflicting command/cmd')
            selector = u32(control.get('command', control.get('cmd')), 'command')
            inner = u32(control.get('status'), 'control.status')
            declared = u32(control.get('params_bytes_declared'), 'params_bytes_declared')
            observed = u32(control.get('params_bytes_observed'), 'params_bytes_observed')
            observed_complete = boolean(control.get('params_complete'), 'params_complete')
            # A retained observation can include serialization padding after the declared bytes.
            require(not observed_complete or observed >= declared, 'truncated native params marked complete')
            incomplete += not observed_complete
        # Current decoder does not decode allocations. Unknown remains unknown.
        entry = event(fn, direction, selector, kind, status, inner)
        generation, trigger = row.get('generation'), row.get('trigger')
        require((generation is None) == (trigger is None), 'generation and trigger must occur together')
        if generation is not None:
            require(u64(generation, 'generation') > 0, 'generation must be positive')
            require(type(trigger) is int and 1 <= trigger <= 4, 'invalid observer trigger')
        entry.update(missing_before=gap, prefix_unknown=prefix, params_complete=observed_complete,
                     generation=generation, trigger=trigger)
        events.append(entry)
        gaps += gap
        unknown += prefix
    export = doc.get('text_export')
    source = None
    if export is not None:
        require(isinstance(export, dict), 'native text_export must be object')
        source = {key: export[key] for key in ('selection', 'omitted_records', 'source_bytes',
                  'source_sha256', 'observations_sha256', 'source_hash_verifiable_from_export') if key in export}
        # Decoder-provided provenance only, not permission to treat selected captures as complete.
        require(len(canonical(source)) <= 4096, 'oversized export provenance')
    stats = doc.get('driver_stats')
    if stats is not None:
        require(isinstance(stats, dict), 'driver_stats must be object')
        allowed = ('triggers', 'bootstrap_attempts', 'attached_tables', 'invalid_bootstrap',
                   'uninitialized_headers', 'invalid_headers', 'read_failures', 'unstable_snapshots',
                   'mapping_changed', 'recorded', 'dropped', 'sequence_gaps', 'invalid_elements',
                   'buffered_bytes')
        stats = {key: (boolean(value, key) if key == 'limited' else u64(value, key))
                 for key, value in stats.items() if key in allowed or key == 'limited'}
    return events, dict(observed_missing=gaps, prefix_unknown_records=unknown,
                        incomplete_control_records=incomplete, sampled=True,
                        source_export=source, driver_stats=stats,
                        attachment_note='Generations are observer attachments, not proven distinct boots. Repeated prefixes are retained. invalid_elements includes repeatedly inspected empty/stale slots, not submitted malformed calls.',
                        request_reply_pairing='not attempted',
                        note='Missing prefix, queue gaps and unobserved traffic remain unknown; retained records may predate collection.')


def projection(entry):
    return {key: entry[key] for key in ('function', 'direction', 'selector_kind', 'selector', 'status', 'control_status')}


def load_run(row, base):
    require(isinstance(row, dict), 'run must be an object')
    run_id = row.get('id')
    require(isinstance(run_id, str) and re.fullmatch(r'[A-Za-z0-9_.-]{1,64}', run_id), 'invalid run id')
    require(row.get('arm') in ('vfio', 'kayfabe'), 'arm must be vfio/kayfabe')
    variant = text(row.get('variant'), 'variant')
    require(isinstance(row.get('flags'), dict), 'run flags must be an explicit object')
    instrumentation = text(row.get('instrumentation'), 'instrumentation')
    metadata = row.get('metadata', {})
    require(isinstance(metadata, dict), 'metadata must be an object')
    for key in IDENTITY_FIELDS:
        if key in metadata:
            text(metadata[key], key)
    run = dict(id=run_id, arm=row['arm'], variant=variant, flags=row['flags'],
               instrumentation=instrumentation, metadata=metadata, rpc=None, display_caps=None)
    for key in ('rpc', 'display_caps'):
        spec = row.get(key)
        if spec is None:
            continue
        require(isinstance(spec, dict), f'{key}: object required')
        path_label = text(spec.get('path'), f'{key}.path')
        path = base / path_label
        data = read_bounded(path, 4096 if key == 'display_caps' else MAX_INPUT)
        artifact = dict(path=path_label, bytes=len(data), sha256=digest(data),
                        note=text(spec.get('note'), f'{key}.note'))
        if key == 'display_caps':
            require(len(data) == 4096, 'display capability page must be exactly 4096 bytes')
            artifact.update(milestone=text(spec.get('milestone'), 'caps.milestone'),
                            source=text(spec.get('source'), 'caps.source'),
                            words=[f'0x{word:08x}' for word in struct.unpack('<1024I', data)])
        else:
            fmt = spec.get('format')
            require(fmt in ('kf-rm', 'kgwt-decoded-json', 'kgwt-decoded-jsonl'), 'unsupported RPC format')
            require((fmt == 'kf-rm') == (row['arm'] == 'kayfabe'), 'RPC format/arm mismatch')
            coverage = spec.get('coverage')
            require(coverage in ('partial', 'complete-window'), 'explicit RPC coverage required')
            require(fmt == 'kf-rm' or coverage == 'partial', 'sampled native coverage must be partial')
            events, details = parse_kf(data) if fmt == 'kf-rm' else parse_native(data, fmt)
            artifact.update(format=fmt, coverage=coverage, window=text(spec.get('window'), 'rpc.window'),
                            events=events, details=details)
        run[key] = artifact
    require(run['rpc'] is not None or run['display_caps'] is not None, 'run has no evidence')
    return run


def sequence_summary(runs, direction):
    sequences = {}
    for run in runs:
        if run['rpc'] is not None:
            sequences[run['id']] = [projection(e) for e in run['rpc']['events'] if e['direction'] == direction]
    hashes = {name: digest(canonical(events).encode()) for name, events in sequences.items()}
    first = next(iter(sequences), None)
    differences = []
    if first is not None:
        baseline = sequences[first]
        for name, events in sequences.items():
            if name == first or events == baseline:
                continue
            index = next((i for i, pair in enumerate(zip(baseline, events)) if pair[0] != pair[1]), min(len(baseline), len(events)))
            differences.append(dict(reference=first, run=name, first_difference=index,
                                    reference_event=baseline[index] if index < len(baseline) else None,
                                    run_event=events[index] if index < len(events) else None))
    counters = {name: Counter(canonical(e) for e in events) for name, events in sequences.items()}
    all_keys = sorted({key for counts in counters.values() for key in counts})
    counts = []
    for key in all_keys:
        values = {run['id']: counters[run['id']][key] if run['id'] in counters else None for run in runs}
        measured = [value for value in values.values() if value is not None]
        counts.append(dict(event=json.loads(key), counts=values, minimum=min(measured), maximum=max(measured),
                           stable=len(measured) >= 2 and len(set(measured)) == 1))
    return dict(direction=direction, observed_runs=len(sequences), sequence_sha256=hashes,
                identical_observed_order=len(sequences) >= 2 and len(set(hashes.values())) == 1,
                first_differences=differences, counts=counts,
                note='Zero means not observed in this supplied capture, not proved absent. Order is filtered observation order, not timing or paired request/reply order.')


def caps_summary(runs):
    rows = []
    for index in range(1024):
        values = {run['id']: run['display_caps']['words'][index] if run['display_caps'] else None for run in runs}
        observed = [v for v in values.values() if v is not None]
        state = ('unavailable' if not observed else 'single-observation' if len(observed) == 1
                 else 'stable' if len(set(observed)) == 1 else 'variable')
        rows.append(dict(offset=f'0x{index * 4:04x}', state=state, values=values))
    return dict(observed_runs=sum(run['display_caps'] is not None for run in runs),
                states=dict(Counter(row['state'] for row in rows)), words=rows)


def cross_rpc(left_runs, right_runs, window):
    """Compare observed selectors, never align messages across different transports."""
    streams = {'vfio_request': (left_runs, 'request'), 'vfio_reply': (left_runs, 'reply'),
               'kayfabe_completed': (right_runs, 'completed')}
    indexed = {}
    keys = set()
    for name, (runs, direction) in streams.items():
        indexed[name] = {}
        for run in runs:
            if run['rpc'] is None:
                indexed[name][run['id']] = None
                continue
            if run['rpc']['window'] != window:
                continue
            counts = {}
            for entry in run['rpc']['events']:
                if entry['direction'] != direction:
                    continue
                # Native allocation classes are not decoded: only fn103 totals are comparable.
                key = (entry['function'], entry['selector'] if entry['function'] == 76 else None)
                keys.add(key)
                statuses = counts.setdefault(key, Counter())
                statuses[(entry['status'], entry['control_status'])] += 1
            indexed[name][run['id']] = counts
    rows = []
    for fn, cmd in sorted(keys, key=lambda key: (key[0], key[1] or '')):
        row = dict(function=fn, control=cmd)
        for name, runs in indexed.items():
            row[name] = {}
            for run_id, counts in runs.items():
                statuses = counts.get((fn, cmd), {}) if counts is not None else None
                row[name][run_id] = None if statuses is None else dict(
                    observed_count=sum(statuses.values()),
                    statuses=[dict(status=outer, control_status=inner, count=count)
                              for (outer, inner), count in sorted(statuses.items(), key=lambda item: canonical(item[0]))])
        rows.append(row)
    return rows


def build_report(manifest_path):
    manifest_path = Path(manifest_path)
    raw = read_bounded(manifest_path, 1024 * 1024)
    doc = parse_json(raw)
    require(isinstance(doc, dict) and doc.get('schema') == 1, 'manifest schema must be 1')
    experiment = doc.get('experiment')
    require(isinstance(experiment, dict), 'experiment object required')
    for key in IDENTITY_FIELDS:
        text(experiment.get(key), f'experiment.{key}')
    entries = doc.get('runs')
    require(isinstance(entries, list) and 1 <= len(entries) <= MAX_RUNS, 'expected 1..32 runs')
    runs, ids, warnings, total_events = [], set(), [], 0
    for entry in entries:
        run = load_run(entry, manifest_path.parent)
        require(run['id'] not in ids, 'duplicate run id')
        ids.add(run['id'])
        total_events += len(run['rpc']['events']) if run['rpc'] else 0
        require(total_events <= MAX_TOTAL_EVENTS, 'aggregate event limit exceeded')
        for key in IDENTITY_FIELDS:
            if key not in run['metadata']:
                warnings.append(f"{run['id']}: {key} inherited from experiment declaration; not independently verified")
            elif run['metadata'][key] != experiment[key]:
                warnings.append(f"{run['id']}: {key} differs from experiment; not a matched-identity run")
        if run['rpc'] is None:
            warnings.append(f"{run['id']}: RPC coverage unavailable")
        if run['display_caps'] is None:
            warnings.append(f"{run['id']}: capability-page coverage unavailable")
        runs.append(run)
    grouped = {}
    for run in runs:
        identity = {key: run['metadata'].get(key, experiment[key]) for key in IDENTITY_FIELDS}
        # Never pool changed instrumentation, flags, identities, milestones or windows.
        key = canonical(dict(arm=run['arm'], variant=run['variant'], flags=run['flags'], identity=identity,
                             instrumentation=run['instrumentation']))
        grouped.setdefault(key, []).append(run)
    groups = []
    for key, group_runs in grouped.items():
        group = json.loads(key)
        group['id'] = f"{group['arm']}-{digest(key.encode())[:12]}"
        group['runs'] = [r['id'] for r in group_runs]
        # Distinct observation windows are kept as independent strata, including missing evidence.
        rpc_windows = sorted({r['rpc']['window'] for r in group_runs if r['rpc']})
        caps_windows = sorted({(r['display_caps']['milestone'], r['display_caps']['source']) for r in group_runs if r['display_caps']})
        group['rpc'] = []
        for window in rpc_windows:
            selected = [r for r in group_runs if r['rpc'] is None or r['rpc']['window'] == window]
            directions = ('completed',) if group['arm'] == 'kayfabe' else ('request', 'reply')
            group['rpc'].append(dict(window=window, runs=[r['id'] for r in selected],
                                     streams=[sequence_summary(selected, d) for d in directions]))
        group['display_caps'] = []
        for milestone, source in caps_windows:
            selected = [r for r in group_runs if r['display_caps'] is None or
                        (r['display_caps']['milestone'], r['display_caps']['source']) == (milestone, source)]
            group['display_caps'].append(dict(milestone=milestone, source=source, **caps_summary(selected)))
        groups.append(group)
    caps_comparisons, rpc_comparisons = [], []
    for left in groups:
        for right in groups:
            if left['arm'] != 'vfio' or right['arm'] != 'kayfabe':
                continue
            common_windows = sorted({w['window'] for w in left['rpc']} & {w['window'] for w in right['rpc']})
            for window in common_windows:
                rpc_comparisons.append(dict(vfio=left['id'], kayfabe=right['id'], window=window,
                    identity_matches=left['identity'] == right['identity'],
                    observations=cross_rpc([r for r in runs if r['id'] in left['runs']],
                                           [r for r in runs if r['id'] in right['runs']], window),
                    note='Observed records only: zero in partial native sampling never establishes absence. Status namespaces and request/reply streams remain separate; allocation-class comparisons are unavailable.'))
            for lc in left['display_caps']:
                for rc in right['display_caps']:
                    if lc['milestone'] != rc['milestone'] or lc['source'] != rc['source']:
                        continue
                    differences = []
                    for lw, rw in zip(lc['words'], rc['words']):
                        if lw['state'] == rw['state'] == 'stable':
                            lv = next(v for v in lw['values'].values() if v is not None)
                            rv = next(v for v in rw['values'].values() if v is not None)
                            if lv != rv:
                                differences.append(dict(offset=lw['offset'], vfio=lv, kayfabe=rv))
                    caps_comparisons.append(dict(vfio=left['id'], kayfabe=right['id'], milestone=lc['milestone'],
                                                 identity_matches=left['identity'] == right['identity'],
                                                 source=lc['source'], stable_differences=differences))
    return dict(schema='kayfabe-boundary-comparison/1', manifest=dict(path=manifest_path.name, sha256=digest(raw)),
                experiment=experiment, warnings=warnings, runs=runs, groups=groups,
                cross_arm_caps=caps_comparisons, cross_arm_rpc=rpc_comparisons,
                interpretation=[
                    'All counts, orders and words are observations of supplied files, not a completeness claim.',
                    'Native KGWT is sampled and partial, including retained history: an absent native call is never proved absent.',
                    'Requests and physical status-queue observations are independent streams. The native direction named reply also includes unsolicited events. No pairing uses handles, QPC, queue addresses or RPC sequence.',
                    'Repeated stability does not imply semantic equivalence, causality, or correctness. Variable values are retained, not classified as noise.',
                    'Different VRAM sizes/topology and diagnostic flags are intentional possibilities; stable cross-arm differences are not automatically defects.',
                    'Manifest identity/milestone declarations are supplied assertions. File hashes authenticate report inputs, not the machine configuration.',
                    'Native allocations lack class decoding here, so class-specific native absence cannot be inferred.',
                    'Numeric outer RPC status, inner control status, and Kayfabe result=none are distinct observations; none is not converted into a numeric error.'
                ])


def md(value):
    return str(value).replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;').replace('|', '&#124;').replace('\n', ' ').replace('\r', ' ').replace('`', '&#96;')


def markdown(report):
    out = ['# Repeated Windows boundary observations', '', '**STATUS: RESEARCH.** Offline comparison; no product-policy changes.', '',
           f"Manifest SHA256: `{report['manifest']['sha256']}`", '',
           '## Observations', '', '| Run | Arm / variant | RPC records / coverage | Capability page SHA256 |', '|---|---|---|---|']
    for run in report['runs']:
        rpc = f"{len(run['rpc']['events'])} / {run['rpc']['coverage']}" if run['rpc'] else 'unavailable'
        caps = run['display_caps']['sha256'] if run['display_caps'] else 'unavailable'
        out.append(f"| {md(run['id'])} | {md(run['arm'])} / {md(run['variant'])} | {rpc} | {caps} |")
    for group in report['groups']:
        out += ['', f"### {md(group['id'])}: {md(group['variant'])}", '', f"Runs: {', '.join(md(r) for r in group['runs'])}."]
        for window in group['rpc']:
            for stream in window['streams']:
                out += ['', f"{md(window['window'])}, {stream['direction']}: {stream['observed_runs']} observed runs; identical normalized order: **{stream['identical_observed_order']}**."]
                changing = [row for row in stream['counts'] if row['minimum'] != row['maximum']]
                out.append(f"{len(changing)} event/status count rows vary. Full counts, first divergences, normalized events and order hashes are in JSON.")
                for diff in stream['first_differences'][:8]:
                    out.append(f"- {md(diff['run'])} first differs from {md(diff['reference'])} at observed index {diff['first_difference']}.")
        for caps in group['display_caps']:
            out += ['', f"Caps at {md(caps['milestone'])}: {caps['observed_runs']} observations; {md(canonical(caps['states']))}."]
            varied = [r for r in caps['words'] if r['state'] == 'variable']
            if varied:
                out += ['', '| Variable offset | All observed values |', '|---|---|']
                for row in varied[:32]:
                    out.append(f"| {row['offset']} | {md(canonical(row['values']))} |")
                if len(varied) > 32:
                    out.append(f"\nShowing 32 of {len(varied)} variable words; all remain in JSON.")
    for comparison in report['cross_arm_caps']:
        out += ['', f"### Stable cross-arm capability differences: {md(comparison['vfio'])} / {md(comparison['kayfabe'])}", '',
                f"At {md(comparison['milestone'])}: {len(comparison['stable_differences'])} words differ. Declared identity matches: {comparison['identity_matches']}.", '',
                '| Offset | VFIO | Kayfabe |', '|---|---|---|']
        for row in comparison['stable_differences'][:64]:
            out.append(f"| {row['offset']} | {row['vfio']} | {row['kayfabe']} |")
        if len(comparison['stable_differences']) > 64:
            out.append('\nFirst 64 shown; every word and per-run value remains in JSON.')
    for comparison in report['cross_arm_rpc']:
        out += ['', f"### Observed cross-arm RPCs: {md(comparison['vfio'])} / {md(comparison['kayfabe'])}", '',
                f"Window {md(comparison['window'])}: {len(comparison['observations'])} function/control rows. Counts and separate outer/inner statuses per run are in JSON.",
                md(comparison['note'])]
    out += ['', '## Interpretation limits', ''] + ['- ' + md(note) for note in report['interpretation']]
    if report['warnings']:
        out += ['', '## Coverage and provenance notes', ''] + ['- ' + md(warning) for warning in report['warnings']]
    out += ['', 'Exact input hashes, instrumentation, variant flags and every normalized observation are in the JSON report.', '']
    return '\n'.join(out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--json', required=True, type=Path, dest='json_path')
    parser.add_argument('--markdown', required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = build_report(args.manifest)
        inputs = {args.manifest.resolve()}
        for run in report['runs']:
            for key in ('rpc', 'display_caps'):
                if run[key]:
                    inputs.add((args.manifest.parent / run[key]['path']).resolve())
        outputs = [args.json_path.resolve(), args.markdown.resolve()]
        require(len(set(outputs)) == 2 and not inputs.intersection(outputs), 'outputs must differ and cannot overwrite evidence')
        # Refuse existing outputs, including dangling symlinks. Evidence stays immutable.
        require(all(not p.exists() and not p.is_symlink() for p in (args.json_path, args.markdown)), 'output already exists')
        with args.json_path.open('x', encoding='utf-8') as stream:
            json.dump(report, stream, indent=2, sort_keys=True)
            stream.write('\n')
        with args.markdown.open('x', encoding='utf-8') as stream:
            stream.write(markdown(report))
    except (InvalidEvidence, OSError) as error:
        parser.exit(2, f'error: {error}\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
