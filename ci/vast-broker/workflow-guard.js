import { reuseReplayStep } from './reuse-bootstrap.js';
import { runnerBootstrap, sweepBootstrap } from './workflow-bootstrap.js';
// Security-focused reader for the repository's indentation-based workflow subset.
// Unsupported dynamic permissions/dependencies fail closed; this is not a YAML evaluator.
import { readdir, readFile } from 'node:fs/promises';
const pagesCondition =
  "${{ github.event_name != 'pull_request' && " +
  "(github.ref == 'refs/heads/master' || startsWith(github.ref, 'refs/tags/v')) }}";

function pagesBoundary(text, jobs) {
  const build = jobs.find((j) => j.id === 'build'),
    deploy = jobs.find((j) => j.id === 'deploy'),
    policy = jobs.find((j) => j.id === 'policy');
  const same = (value, expected) =>
    JSON.stringify(Object.entries(value ?? {}).sort()) ===
    JSON.stringify(Object.entries(expected).sort());
  const condition = deploy?.body.match(/^    if: >-\n((?:      .*\n?)+)/m)?.[1] ?? '';
  const expectedCondition = policy
    ? pagesCondition.slice(0, -3) + " && needs.policy.outputs.deploy == '1' }}"
    : pagesCondition;
  const pr = text.split('  pull_request:\n')[1]?.split(/^  \S/m)[0] ?? '';
  if (
    jobs.length !== (policy ? 3 : 2) ||
    !build ||
    !deploy ||
    (policy &&
      (!same(policy.grants, { contents: 'read', 'pull-requests': 'read' }) ||
        /\buses:|\bsecrets\./.test(policy.body) ||
        /\bpaths(?:-ignore)?:/.test(pr))) ||
    !same(build.grants, { contents: 'read' }) ||
    !same(deploy.grants, { pages: 'write', 'id-token': 'write' }) ||
    JSON.stringify(deploy.needs) !== JSON.stringify(policy ? ['policy', 'build'] : ['build']) ||
    condition.replace(/\s/g, '') !== expectedCondition.replace(/\s/g, '') ||
    !/^    environment:\n      name: github-pages\s*$/m.test(deploy.body) ||
    !/^  pull_request:/m.test(text) ||
    /pull_request_target:|\bsecrets\./.test(text) ||
    [...text.matchAll(/\buses:\s*(\S+)/g)].some((m) => !/@[0-9a-f]{40}$/.test(m[1]))
  )
    return ['site.yml: requires read-only path/build jobs and guarded, pinned Pages-only deploy'];
  return [];
}
function siteStatusBoundary(text, jobs) {
  const job = jobs[0];
  const grants = {
    contents: 'read',
    'pull-requests': 'read',
    actions: 'read',
    checks: 'read',
    statuses: 'write',
  };
  const events = text.split('on:\n')[1]?.split(/^\S/m)[0] ?? '';
  const actual = job?.body
    .split('    steps:\n')[1]
    ?.split('\n')
    .map((line) => (line.startsWith('      ') ? line.slice(6) : line))
    .join('\n')
    .trim();
  const expected = [
    '- name: Report Site metadata without running PR code',
    '  env:',
    '    GH_TOKEN: ${{ github.token }}',
    '  run: |',
    '    set -euo pipefail',
    '    [[ "$GITHUB_SHA" =~ ^[0-9a-f]{40}$ ]]',
    '    temporary=$(mktemp -d)',
    '    trap \'find "$temporary" -depth -delete\' EXIT',
    '    source_origin=https://raw.githubusercontent.com/reindertpelsma/kayfabe',
    "    curl --silent --fail --proto '=https' --max-time 20 --max-filesize 131072 \\",
    '      -o "$temporary/site_policy.py" \\',
    '      "$source_origin/$GITHUB_SHA/ci/vast-broker/site_policy.py"',
    '    python3 -I "$temporary/site_policy.py" status',
  ].join('\n');
  if (
    jobs.length !== 1 ||
    job?.id !== 'status' ||
    actual !== expected ||
    JSON.stringify(Object.entries(job?.grants ?? {}).sort()) !==
      JSON.stringify(Object.entries(grants).sort()) ||
    JSON.stringify([...events.matchAll(/^  (\w+):/gm)].map((match) => match[1])) !==
      JSON.stringify(['pull_request_target', 'workflow_run']) ||
    !events.includes('    workflows: [Site]') ||
    !events.includes('    types: [in_progress, completed]') ||
    /\buses:|\bsecrets\.|\bid-token:|\bpages:/.test(text)
  )
    return ['site-status.yml: requires the exact API-only trusted status reporter'];
  return [];
}
function jobsIn(text) {
  const lines = text.split('\n');
  const jobs = [];
  let inside = false,
    current;
  for (const line of lines) {
    if (line === 'jobs:') {
      inside = true;
      continue;
    }
    if (!inside) continue;
    if (/^\S/.test(line) && !line.startsWith('#')) {
      inside = false;
      continue;
    }
    const match = /^  ([\w-]+):\s*(?:#.*)?$/.exec(line);
    if (match) {
      current = { id: match[1], lines: [] };
      jobs.push(current);
    } else if (/^  [^\s#].*:/.test(line)) {
      current = { id: 'unsupported-' + jobs.length, lines: [], unsupported: true };
      jobs.push(current);
    } else if (current) current.lines.push(line);
  }
  return jobs.map((job) => {
    const body = job.lines.join('\n');
    const needs = [];
    let dependency = false,
      permissions = false;
    const grants = {};
    let unsupported = job.unsupported === true;
    for (const line of job.lines) {
      if (/^    needs:/.test(line)) {
        const v = line
          .replace(/^    needs:\s*/, '')
          .replace(/#.*/, '')
          .trim();
        dependency = !v;
        if (v) {
          if (!/^[\w\s,\[\]'"-]+$/.test(v)) unsupported = true;
          else
            needs.push(
              ...v
                .replace(/[\[\]'",]/g, ' ')
                .split(/\s+/)
                .filter(Boolean),
            );
        }
        continue;
      }
      if (dependency) {
        const m = /^      - ([\w-]+)\s*$/.exec(line);
        if (m) {
          needs.push(m[1]);
          continue;
        }
        if (/^    \S/.test(line)) dependency = false;
        else if (line.trim()) unsupported = true;
      }
      if (/^    permissions:/.test(line)) {
        permissions = true;
        if (line.trim() !== 'permissions:') unsupported = true;
        continue;
      }
      if (permissions) {
        const m = /^      ([\w-]+): (read|write|none)\s*$/.exec(line);
        if (m) {
          grants[m[1]] = m[2];
          continue;
        }
        if (/^    \S/.test(line)) permissions = false;
        else if (line.trim() && !line.trim().startsWith('#')) unsupported = true;
      }
    }
    const danger =
      /attest|release|publish/i.test(job.id) ||
      /uses:.*(?:attest|release)|gh\s+(?:attestation|release)\s|attestations:\s*write/i.test(body);
    return { ...job, body, needs, grants, unsupported, danger, opaque: /^    uses:/m.test(body) };
  });
}
function endpointBoundary(filename, text, jobs) {
  const accessAlias = /\bCF_ACCESS_CLIENT_(?:ID|SECRET)\b/;
  const endpointBindings = {
    WORKER_URL: 'vars.VAST_BROKER_URL',
    WORKER_PATH: 'secrets.VAST_BROKER_PATH',
    CF_ACCESS_CLIENT_ID: 'secrets.VAST_BROKER_ACCESS_ID',
    CF_ACCESS_CLIENT_SECRET: 'secrets.VAST_BROKER_ACCESS_SECRET',
  };
  const errors = [],
    fail = (reason) => errors.push('Broker endpoint ' + reason);
  if (/^[ \t]*['"]?secrets['"]?[ \t]*:[ \t]*[^#\s]/m.test(text))
    fail('secret forwarding requires explicit block mappings');
  const primary = ['gpu-vast.yml', 'gpu-vast-sweep.yml'].includes(filename)
    ? jobs.find((x) => x.id === 'gpu')
    : null;
  let remaining = text,
    outside = text;
  if (primary) {
    const parts = primary.body.split(/(?=^      - )/m),
      allSteps = parts.slice(1),
      steps = allSteps.filter((step) => /^        id: broker\s*$/m.test(step)),
      replay = allSteps.length === 2 && allSteps[0].trim() === reuseReplayStep.trim();
    if (
      steps.length !== 1 ||
      !(allSteps.length === 1 || replay) ||
      (replay && !/^        if: needs.pending.outputs.reused != 'true'$/m.test(steps[0]))
    )
      fail('secrets require the sole broker step and an exact read-only replay');
    const header = parts[0];
    if (/\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE)\b/.test(header))
      fail('secrets cannot enter job env or outputs');
    const output = header.split('    outputs:\n')[1]?.split(/^    \S/m)[0];
    if (
      output &&
      output.trim() !==
        'state: ${{ steps.broker.outputs.state }}\n      verdict: ${{ steps.broker.outputs.verdict }}\n      verified: ${{ steps.broker.outputs.verified }}'
    )
      fail('job outputs must be fixed state and verdict');
    let sanitised = header;
    for (const step of steps) {
      if (!/^        run: \|[ \t]*$/m.test(step)) fail('requires literal broker run syntax');
      let environment = false;
      const found = new Set(),
        kept = [];
      for (const line of step.split('\n')) {
        if (/^        env:\s*$/.test(line)) environment = true;
        else if (/^        \S/.test(line)) environment = false;
        const match = /^          ([A-Z_]+): \$\{\{ ((?:vars|secrets)\.[A-Z_]+) \}\}$/.exec(line);
        if (
          environment &&
          steps.length === 1 &&
          /^        id: broker\s*$/m.test(step) &&
          match &&
          endpointBindings[match[1]] === match[2]
        ) {
          found.add(match[1]);
          continue;
        }
        kept.push(line);
      }
      if (!['WORKER_URL', 'WORKER_PATH'].every((x) => found.has(x)))
        fail('requires public URL and secret path step-env bindings');
      if (
        /\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE|broker_origin|broker_host|broker_base|audience)\b/.test(
          kept.join('\n').split(/^        run:/m)[0],
        )
      )
        fail('aliases cannot enter step metadata');
      const run = step.split(/^        run: \|\s*$/m)[1] || '';
      const approvedRenderer =
        run
          .split('\n')
          .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
          .join('\n')
          .trim() === (filename === 'gpu-vast-sweep.yml' ? sweepBootstrap : runnerBootstrap).trim();
      if (!approvedRenderer) fail('requires exact trusted BASE controller bootstrap');
      for (const line of run.split('\n')) {
        if (/GITHUB_ENV/.test(line)) fail('cannot enter environment channels');
        if (
          /GITHUB_STEP_SUMMARY|GITHUB_OUTPUT/.test(line) &&
          !(
            approvedRenderer &&
            line.trim() ===
              'GITHUB_OUTPUT="$GITHUB_OUTPUT" GITHUB_STEP_SUMMARY="$GITHUB_STEP_SUMMARY" \\'
          )
        )
          fail('only BASE controller may write summaries/enum outputs');
        if (
          /^\s*(?:echo|printf|cat|tee)\b/.test(line) &&
          /\b(?:WORKER_URL|WORKER_PATH|broker_origin|broker_host|broker_base|host_parts|audience)\b/.test(
            line,
          )
        )
          fail('cannot be printed');
      }
      sanitised += kept.join('\n');
    }
    remaining = text.replace(primary.body, sanitised);
    outside = text.replace(primary.body, '');
  }
  if (/VAST_BROKER_(?:URL|PATH|ACCESS_ID|ACCESS_SECRET)/.test(remaining))
    fail('secret reference outside approved step env');
  if (/\b(?:WORKER_URL|WORKER_PATH|BROKER_BASE)\b/.test(outside))
    fail('aliases outside broker step');
  for (const expression of text.match(/\$\{\{[\s\S]*?\}\}/g) || [])
    if (/\bsecrets\b(?!\s*\.)/.test(expression)) fail('opaque secret context is unsupported');
  return errors;
}
export function checkWorkflows(workflows) {
  const errors = [];
  const gpuName = 'gpu-vast.yml';
  if (!workflows[gpuName]) return ['GPU workflow missing'];
  const gpu = jobsIn(workflows[gpuName]);
  if (
    !/^  pull_request_target:/m.test(workflows[gpuName]) ||
    /^  pull_request:/m.test(workflows[gpuName]) ||
    !workflows[gpuName].includes('bash --noprofile --norc -eo pipefail {0}') ||
    !workflows[gpuName].includes('concurrency:')
  )
    errors.push('GPU runner must use trusted base trigger, fixed shell and head concurrency');
  if (/actions\/checkout|cache@|upload-artifact|download-artifact/.test(workflows[gpuName]))
    errors.push('GPU runner cannot consume source/artifact/cache actions');

  if (!gpu.some((x) => x.id === 'pending') || !gpu.some((x) => x.id === 'report'))
    errors.push('GPU status writers missing');
  const report = gpu.find((x) => x.id === 'report');
  if (
    report &&
    (!report.body.includes('    if: ${{ always() }}') ||
      !['pending', 'gpu'].every((id) => report.needs.includes(id)))
  )
    errors.push('Final status must always evaluate both dependencies');
  const primary = gpu.find((x) => x.id === 'gpu');
  if (!primary) errors.push('GPU poll job missing');
  if (
    primary &&
    JSON.stringify(Object.entries(primary.grants).sort()) !==
      JSON.stringify(Object.entries({ 'id-token': 'write' }).sort())
  )
    errors.push('GPU permissions must be exactly id-token/write');
  for (const job of gpu.filter((x) => ['pending', 'report'].includes(x.id))) {
    if (
      ![
        { statuses: 'write' },
        ...(job.id === 'pending'
          ? [
              {
                statuses: 'write',
                contents: 'read',
                'pull-requests': 'read',
                actions: 'read',
                checks: 'read',
              },
            ]
          : []),
      ].some(
        (expected) =>
          JSON.stringify(Object.entries(job.grants).sort()) ===
          JSON.stringify(Object.entries(expected).sort()),
      ) ||
      /X-GH-Token|WORKER_URL|id-token:/.test(job.body)
    )
      errors.push('Status writer must not send its token to Worker');
  }
  for (const [filename, text] of Object.entries(workflows)) {
    if (filename === 'gpu-vast-sweep.yml') {
      if (
        !/^  workflow_dispatch:/m.test(text) ||
        /^  (?:pull_request|pull_request_target|push|schedule):/m.test(text) ||
        /statuses:|context:[ "]*gpu-vast|actions\/checkout|cache@|upload-artifact|download-artifact/.test(
          text,
        )
      )
        errors.push(
          'Sweep must be a dispatch-only finding without merge status or source/artifact input',
        );
      const job = jobsIn(text);
      if (
        job.length !== 1 ||
        job[0].id !== 'gpu' ||
        JSON.stringify(Object.entries(job[0].grants)) !== JSON.stringify([['id-token', 'write']])
      )
        errors.push('Sweep must have one OIDC-only broker job');
    }
    const jobs = jobsIn(text),
      map = new Map(jobs.map((j) => [j.id, j]));
    if (filename === 'site.yml') errors.push(...pagesBoundary(text, jobs));
    if (filename === 'site-status.yml') errors.push(...siteStatusBoundary(text, jobs));
    let outsideJobs = text;
    for (const job of jobs) {
      outsideJobs = outsideJobs.replace(job.body, '');
      const gpuOidc = ['gpu-vast.yml', 'gpu-vast-sweep.yml'].includes(filename) && job.id === 'gpu';
      const releaseOidc = filename === 'release-build.yml' && job.id === 'build';
      const pagesOidc = filename === 'site.yml' && job.id === 'deploy';
      if (job.grants['id-token'] === 'write' && !gpuOidc && !releaseOidc && !pagesOidc)
        errors.push(filename + ': OIDC outside approved GPU/release/Pages job');
      if (job.grants.pages === 'write' && !pagesOidc)
        errors.push(filename + ': Pages write outside site deploy');
    }
    if (/id-token:\s*write/.test(outsideJobs))
      errors.push(filename + ': workflow-level OIDC permission is forbidden');
    errors.push(...endpointBoundary(filename, text, jobs));
    const reaches = (job, seen = new Set()) => {
      if (seen.has(job.id)) return false;
      seen.add(job.id);
      return job.needs.some(
        (n) => gpu.some((g) => g.id === n) || (map.has(n) && reaches(map.get(n), seen)),
      );
    };
    for (const job of jobs) {
      if (job.unsupported)
        errors.push(filename + ': unsupported dynamic permission/needs in ' + job.id);
      if (filename === gpuName && job.opaque)
        errors.push('GPU workflow cannot call opaque reusable workflow');
      if (job.danger && (filename === gpuName || reaches(job)))
        errors.push(filename + ': attestation/release consumes GPU workflow');
    }
    if (
      filename !== gpuName &&
      /workflow_run:/.test(text) &&
      /Vast GPU|gpu-vast|workflows:\s*\[?\s*['"]?\*/.test(text)
    )
      errors.push(filename + ': downstream workflow_run GPU consumer');
  }
  if (/merge_group:|upload-artifact|download-artifact/.test(workflows[gpuName]))
    errors.push('GPU workflow has forbidden trigger/artifact channel');
  return errors;
}
export async function checkRepository(root) {
  const folder = new URL('.github/workflows/', root),
    workflows = {};
  for (const name of await readdir(folder))
    if (/\.ya?ml$/.test(name)) workflows[name] = await readFile(new URL(name, folder), 'utf8');
  return checkWorkflows(workflows);
}
