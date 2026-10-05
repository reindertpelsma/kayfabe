#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded, payload-free native attachment/status census against Kayfabe observations.

Input is the normal comparator manifest plus source{values,version,commit}. Values is
the compiler-produced values.tsv from source.spec. The trusted observer decoder must
already have validated the private native capture. This tool never pairs messages.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import re

import compare

MAX_CONTROLS = 1024
MAX_ATTACHMENTS = 64


def source_map(spec, base):
    compare.require(isinstance(spec, dict), 'source object required')
    path = base / compare.text(spec.get('values'), 'source.values')
    raw = compare.read_bounded(path, 2 * 1024 * 1024)
    maps = {key: defaultdict(list) for key in ('rpc_functions', 'rpc_events', 'ctrl_cmds')}
    for line in raw.decode('ascii').splitlines():
        parts = line.split('\t')
        compare.require(len(parts) == 3 and parts[0] in maps, 'invalid compiler values row')
        group, name, value = parts
        compare.require(re.fullmatch(r'[A-Z][A-Z0-9_]{0,191}', name), 'invalid compiler symbol')
        numeric = compare.u32(value, 'compiler value')
        # Enum sentinels bound a vocabulary; they do not denote submitted events.
        if name in ('NV_VGPU_MSG_EVENT_FIRST_EVENT', 'NV_VGPU_MSG_EVENT_NUM_EVENTS',
                    'NV_VGPU_MSG_FUNCTION_NUM_FUNCTIONS'):
            continue
        if name not in maps[group][numeric]:
            maps[group][numeric].append(name)
    compare.require(maps['rpc_functions'] and maps['rpc_events'] and maps['ctrl_cmds'],
                    'incomplete compiler vocabulary')
    return maps, dict(version=compare.text(spec.get('version'), 'source.version'),
                      commit=compare.text(spec.get('commit'), 'source.commit'),
                      values_sha256=compare.digest(raw))


def stream_kind(event, maps):
    if event['direction'] == 'request':
        return 'request'
    if event['function'] in maps['rpc_events']:
        return 'asynchronous_status_queue'
    if event['function'] in maps['rpc_functions']:
        return 'rpc_status_queue'
    return 'unknown_status_queue'


def counts(events):
    rows = Counter(compare.canonical(compare.projection(event)) for event in events)
    return [dict(event=json.loads(key), count=value) for key, value in sorted(rows.items())]


def sequence(events):
    return compare.digest(compare.canonical([compare.projection(e) for e in events]).encode())


def native_summary(run, maps):
    events = run['rpc']['events']
    generations = defaultdict(list)
    for index, event in enumerate(events):
        generations[event['generation']].append((index, event))
    compare.require(len(generations) <= MAX_ATTACHMENTS, 'more than 64 attachment generations')
    attachments = []
    for generation, indexed in generations.items():
        streams = {}
        for kind in ('request', 'rpc_status_queue', 'asynchronous_status_queue', 'unknown_status_queue'):
            selected = [e for _, e in indexed if stream_kind(e, maps) == kind]
            streams[kind] = dict(records=len(selected), sequence_sha256=sequence(selected),
                                 counts=counts(selected))
        attachments.append(dict(generation=generation, records=len(indexed),
                                first_observation=indexed[0][0], last_observation=indexed[-1][0],
                                triggers=dict(Counter(str(e['trigger']) for _, e in indexed)), streams=streams))
    return dict(records=len(events), attachment_generations=attachments,
                coverage=run['rpc']['coverage'], details=run['rpc']['details'])


def control_statuses(events, command, native):
    indexed = [(i, e) for i, e in enumerate(events) if e['function'] == 76 and e['selector'] == command
               and e['direction'] == ('reply' if native else 'completed')]
    observed = Counter((e['status'], e['control_status'] if native else None) for _, e in indexed)
    return dict(observed_count=len(indexed), first_observation=indexed[0][0] if indexed else None,
                statuses=[dict(outer_or_result=outer, control_status=inner, count=n)
                          for (outer, inner), n in sorted(observed.items(), key=lambda item: compare.canonical(item[0]))],
                observed_control_success=any(e['status'] == e['control_status'] == '0x00000000'
                                             for _, e in indexed) if native else None)


def control_index(events, native):
    """Index once: large supplied logs must not cause a quadratic per-command scan."""
    indexed = defaultdict(list)
    for index, event in enumerate(events):
        if event['function'] == 76 and event['selector'] is not None and event['direction'] == ('reply' if native else 'completed'):
            indexed[event['selector']].append((index, event))
    result = {}
    for command, items in indexed.items():
        summary = control_statuses([event for _, event in items], command, native)
        summary['first_observation'] = items[0][0]
        result[command] = summary
    return result


def repeated_native(runs, maps):
    groups = defaultdict(list)
    for run in runs:
        identity = {key: run['metadata'].get(key) for key in compare.IDENTITY_FIELDS + ('qemu_sha256', 'qemu_artifact_revision')}
        key = compare.canonical(dict(identity=identity, variant=run['variant'], flags=run['flags'],
                                     instrumentation=run['instrumentation'], window=run['rpc']['window']))
        groups[key].append(run)
    result = []
    for key, selected in groups.items():
        generations = sorted({e['generation'] for run in selected for e in run['rpc']['events']},
                             key=lambda value: -1 if value is None else value)
        strata = []
        for generation in generations:
            for kind in ('request', 'rpc_status_queue', 'asynchronous_status_queue', 'unknown_status_queue'):
                streams = {run['id']: [compare.projection(e) for e in run['rpc']['events']
                                      if e['generation'] == generation and stream_kind(e, maps) == kind]
                           for run in selected}
                prefix = 0
                if len(streams) >= 2:
                    for row in zip(*streams.values()):
                        if any(e != row[0] for e in row[1:]):
                            break
                        prefix += 1
                strata.append(dict(generation=generation, stream=kind,
                                    common_observed_prefix_records=prefix if len(streams) >= 2 else None,
                                    observed_lengths={name:len(events) for name,events in streams.items()}))
        result.append(dict(cohort=json.loads(key), runs=[r['id'] for r in selected], strata=strata,
                           note='Generation numbers are attachment labels within each run; comparisons do not identify boots or prove complete prefixes.'))
    return result


def build(manifest):
    manifest = Path(manifest)
    spec = compare.parse_json(compare.read_bounded(manifest, 1024 * 1024))
    maps, provenance = source_map(spec.get('source'), manifest.parent)
    report = compare.build_report(manifest)
    runs = report['runs']
    native = [r for r in runs if r['arm'] == 'vfio' and r['rpc']]
    kay = [r for r in runs if r['arm'] == 'kayfabe' and r['rpc']]
    compare.require(native and kay, 'at least one native and one Kayfabe RPC input required')
    public_runs = []
    for run in runs:
        rpc = run['rpc']
        row = {key: run[key] for key in ('id', 'arm', 'variant', 'flags', 'instrumentation', 'metadata')}
        row['rpc'] = None
        if rpc:
            row['rpc'] = dict(sha256=rpc['sha256'], bytes=rpc['bytes'], window=rpc['window'],
                              note=rpc['note'], coverage=rpc['coverage'])
            if run['arm'] == 'vfio':
                row['rpc'].update(native_summary(run, maps))
            else:
                events = rpc['events']
                row['rpc'].update(records=len(events), sequence_sha256=sequence(events),
                                  multiset_sha256=compare.digest(compare.canonical(counts(events)).encode()),
                                  counts=counts(events))
        row['caps_sha256'] = run['display_caps']['sha256'] if run['display_caps'] else None
        public_runs.append(row)
    # Restrict the cross-arm census to the observed Kayfabe initialization vocabulary.
    # Native post-init/DWM controls outside this set never become a missing-call list.
    commands = sorted({e['selector'] for r in kay for e in r['rpc']['events']
                       if e['function'] == 76 and e['selector'] is not None})
    compare.require(len(commands) <= MAX_CONTROLS, 'more than 1024 compared control IDs')
    indexed = {r['id']: control_index(r['rpc']['events'], r['arm'] == 'vfio') for r in native + kay}
    empty_native, empty_kay = control_statuses([], None, True), control_statuses([], None, False)
    rows = []
    for command in commands:
        k = {r['id']: indexed[r['id']].get(command, empty_kay) for r in kay}
        n = {r['id']: indexed[r['id']].get(command, empty_native) for r in native}
        rows.append(dict(command=command, public_names=maps['ctrl_cmds'].get(int(command, 16), []),
                         kayfabe=k, native_status_queue=n,
                         kayfabe_declined_or_nonzero=any(
                             s['outer_or_result'] != '0x00000000' for v in k.values() for s in v['statuses']),
                         native_success_observed_in_every_capture=all(v['observed_control_success'] for v in n.values())))
    native_function_names = {str(e['function']): maps['rpc_functions'].get(e['function'], []) +
                             maps['rpc_events'].get(e['function'], [])
                             for r in native for e in r['rpc']['events']}
    return dict(schema='kayfabe-native-cohort/1', manifest=report['manifest'], source=provenance,
                experiment=report['experiment'], warnings=report['warnings'], runs=public_runs,
                function_names=native_function_names, controls_observed_by_kayfabe=rows,
                native_repeat_strata=repeated_native(native, maps),
                interpretation=[
                    'All native coverage remains partial, even with queue sequence zero and zero observed gaps/drops.',
                    'Attachment generations are observer attachment epochs, not proven separate boots. No prefix is deduplicated.',
                    'Physical status-queue direction is called reply by the decoder; source-defined unsolicited events are separated here.',
                    'No request/reply pairing or cross-arm phase alignment is attempted. First-observation indices refer to different streams.',
                    'Native success for the same numeric control ID does not establish matching object, parameters, phase, or required virtual-device behavior.',
                    'Only controls observed by Kayfabe are compared. Other native traffic may include DWM/userspace activity; it is not a kernel implementation backlog.',
                    'Native outer RPC and inner control status differ. Kayfabe result=none stays null; no numeric status is inferred.',
                    'invalid_elements counts sampled empty/stale slots too, not malformed submitted RPCs. Unstable snapshots remain explicit coverage limits.',
                    'Compiler names associate exact source numbers, not runtime policy. No name means unresolved in this source vocabulary.',
                    'Metadata/identity declarations are supplied assertions; compare QEMU executable hashes in addition to revision labels.',
                    'Output excludes raw messages, addresses, handles, QPCs and capability words. Review supplied metadata before publication.'
                ])


def compact(report):
    """Omit per-run full censuses; preserve their hashes and all targeted control rows."""
    for run in report['runs']:
        rpc = run['rpc']
        if rpc is None:
            continue
        censuses = [rpc] if run['arm'] == 'kayfabe' else [stream for group in rpc['attachment_generations']
                                                        for stream in group['streams'].values()]
        for census in censuses:
            rows = census.pop('counts')
            census['count_rows'] = len(rows)
            census['counts_sha256'] = compare.digest(compare.canonical(rows).encode())
    return report


def markdown(report):
    out = ['# Native Windows boundary cohort', '', '**STATUS: RESEARCH, 2026-10-05.**', '',
           '| Run | Records | Attachments | Gaps / drops / unstable |', '|---|---:|---|---|']
    for run in report['runs']:
        rpc = run['rpc']
        if not rpc:
            continue
        generations = rpc.get('attachment_generations')
        attachments = ', '.join(f"{g['generation']}:{g['records']}" for g in generations) if generations else 'n/a'
        stats = rpc.get('details', {}).get('driver_stats') or {}
        health = ' / '.join(str(stats.get(key, 'unavailable')) for key in ('sequence_gaps', 'dropped', 'unstable_snapshots'))
        out.append(f"| {compare.md(run['id'])} | {rpc['records']} | {attachments} | {health} |")
    out += ['', '## Observed declined or nonzero Kayfabe controls', '',
            'Native columns list observed **inner control** statuses and counts; these are not paired replies or matched inputs.', '',
            '| ID / public source name | Kayfabe results | Native status observations |', '|---|---|---|']
    for row in report['controls_observed_by_kayfabe']:
        if not row['kayfabe_declined_or_nonzero']:
            continue
        name = ', '.join(row['public_names']) or 'not named by this source vocabulary'
        def cells(key, field):
            return '; '.join(compare.md(run) + ': ' + ', '.join(
                f"{s[field] if s[field] is not None else 'none'} ×{s['count']}" for s in value['statuses'])
                for run, value in row[key].items())
        out.append(f"| {row['command']} {compare.md(name)} | {cells('kayfabe', 'outer_or_result')} | {cells('native_status_queue', 'control_status')} |")
    out += ['', '## Interpretation and limits', ''] + ['- ' + item for item in report['interpretation']]
    return '\n'.join(out) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('--json', type=Path, required=True)
    parser.add_argument('--markdown', type=Path, required=True)
    parser.add_argument('--compact', action='store_true', help='omit full native per-run command censuses; retain hashes and targeted controls')
    args = parser.parse_args()
    compare.require(args.json.resolve() != args.markdown.resolve(), 'outputs must differ')
    compare.require(not args.json.exists() and not args.markdown.exists(), 'refusing existing output')
    report = build(args.manifest)
    if args.compact:
        compact(report)
    with args.json.open('x') as out:
        out.write(json.dumps(report, indent=2, sort_keys=True) + '\n')
    with args.markdown.open('x') as out:
        out.write(markdown(report))


if __name__ == '__main__':
    try:
        main()
    except (compare.InvalidEvidence, OSError, UnicodeError) as error:
        raise SystemExit(str(error)) from error
