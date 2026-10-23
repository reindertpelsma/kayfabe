// Trusted BASE workflow policy only. Uncertain metadata always requests the GPU check.
import { readFile, appendFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { Github } from './src/github.js';
import { hexSha, skippedFiles } from './src/policy.js';
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
    const github = new Github(fetcher, { repo, patterns }, token);
    if (eventName === 'pull_request_target') {
      if (
        !Number.isSafeInteger(event?.number) ||
        event.number <= 0 ||
        !hexSha(event.pull_request?.head?.sha)
      )
        return true;
      const pr = await github.pr(event.number);
      if (
        pr.state !== 'open' ||
        pr.head?.sha !== event.pull_request.head.sha ||
        pr.head?.repo?.full_name !== repo
      )
        return true;
      if (!(await github.allSkipped(pr))) return true;
      const current = await github.pr(event.number);
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
