// Trusted BASE workflow policy only. Uncertain metadata always requests the GPU check.
import { readFile, appendFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { Github } from './src/github.js';
import { hexSha } from './src/policy.js';
// Exclusions apply only to canonical API paths. Unknown names are always relevant.
export function skippedPath(path, patterns) {
  if (
    typeof path !== 'string' ||
    !path ||
    path.length > 4096 ||
    !/^[A-Za-z0-9_ ./-]+$/.test(path) ||
    path.split('/').some((x) => !x || x === '.' || x === '..') ||
    path.startsWith('ci/vast-broker/')
  )
    return false;
  return patterns.some((pattern) => {
    let expression = '^';
    for (let i = 0; i < pattern.length; i++) {
      const char = pattern[i];
      if (char === '*' && pattern[i + 1] === '*') {
        i++;
        if (pattern[i + 1] === '/') {
          i++;
          expression += '(?:.*/)?';
        } else expression += '.*';
      } else if (char === '*') expression += '[^/]*';
      else if (char === '?') expression += '[^/]';
      else expression += char === '.' ? '\\.' : char;
    }
    return new RegExp(expression + '$').test(path);
  });
}
export function skippedFiles(files, count, patterns) {
  if (
    !Number.isSafeInteger(count) ||
    count <= 0 ||
    count >= 3000 ||
    !Array.isArray(files) ||
    files.length !== count ||
    new Set(files.map((x) => x?.filename)).size !== count
  )
    return false;
  return files.every(
    (x) =>
      x &&
      ['added', 'modified', 'removed', 'renamed', 'copied', 'changed'].includes(x.status) &&
      skippedPath(x.filename, patterns) &&
      (!['renamed', 'copied'].includes(x.status) || typeof x.previous_filename === 'string') &&
      (x.previous_filename === undefined || skippedPath(x.previous_filename, patterns)),
  );
}
// Complete PR file metadata only; any truncation or mismatch means "run".
async function allSkipped(github, p, patterns) {
  const count = p.changed_files;
  if (!Number.isSafeInteger(count) || count <= 0 || count >= 3000) return false;
  const files = [];
  for (let page = 1; page <= Math.ceil(count / 100); page++) {
    const list = await github.get(`pulls/${p.number}/files?per_page=100&page=${page}`);
    if (!Array.isArray(list) || list.length !== Math.min(100, count - files.length)) return false;
    files.push(...list);
  }
  return skippedFiles(files, count, patterns);
}
export async function shouldRunTrigger({
  eventName,
  event,
  repo,
  patterns,
  token,
  fetcher = fetch,
}) {
  try {
    if (repo !== 'reindertpelsma/kayfabe' || !Array.isArray(patterns)) return true;
    const github = new Github(fetcher, { repo }, token);
    if (eventName === 'pull_request_target') {
      if (
        !Number.isSafeInteger(event?.number) ||
        event.number <= 0 ||
        !hexSha(event.pull_request?.head?.sha)
      )
        return true;
      const pr = await github.get('pulls/' + event.number);
      if (
        pr.state !== 'open' ||
        pr.head?.sha !== event.pull_request.head.sha ||
        pr.head?.repo?.full_name !== repo
      )
        return true;
      if (!(await allSkipped(github, pr, patterns))) return true;
      const current = await github.get('pulls/' + event.number);
      return current.state !== 'open' || current.head?.sha !== pr.head.sha;
    }
    if (eventName === 'push') {
      if (!hexSha(event?.before) || !hexSha(event?.after)) return true;
      const comparison = await github.get(
        'compare/' + event.before + '...' + event.after + '?per_page=1&page=1',
      );
      if (
        !['ahead', 'identical'].includes(comparison.status) ||
        comparison.base_commit?.sha !== event.before ||
        comparison.merge_base_commit?.sha !== event.before ||
        !Array.isArray(comparison.files) ||
        !comparison.files.length ||
        comparison.files.length >= 300
      )
        return true;
      return !skippedFiles(comparison.files, comparison.files.length, patterns);
    }
    return true;
  } catch {
    return true;
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  let run = true;
  try {
    if ((await stat(process.env.GITHUB_EVENT_PATH)).size <= 262144) {
      run = await shouldRunTrigger({
        eventName: process.env.GITHUB_EVENT_NAME,
        event: JSON.parse(await readFile(process.env.GITHUB_EVENT_PATH, 'utf8')),
        repo: process.env.GITHUB_REPOSITORY,
        patterns: JSON.parse(process.env.PATH_SKIP_PATTERNS ?? '[]'),
        token: process.env.GH_TOKEN,
      });
    }
  } catch {} // Never render untrusted paths, upstream bodies or credentials.
  await appendFile(process.env.GITHUB_OUTPUT, 'run=' + (run ? '1' : '0') + '\n');
}
