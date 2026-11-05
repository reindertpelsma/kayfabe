import { fetchBounded } from './policy.js';
// The result key of a commit: a hash of its tree after the commit's own .workerignore.
// Design: docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md §3 (AGREED RECOMMENDATION, 2026-11-05).
//
// key = SHA-256 over 'kf-tree-v1\n' + for every unignored file, sorted by the UTF-8 bytes of its
//       path: path NUL mode NUL object-sha LF. Files are blobs (100644, 100755), symlinks
//       (120000) and submodules (160000); directories are not entries.
// Commits with identical filtered trees share every result. A commit that newly ignores a file
// removes it from the list, so its key changes; a new file added together with its exclusion
// never enters the list, so the key stays.
//
// FAIL CLOSED: any pattern outside the subset below, a truncated or oversized tree listing, an
// unreadable .workerignore or a GitHub error returns null, which means "run, no fall-through".
//
// .workerignore syntax: a subset that git reads identically (test/treekey-parity.test.js checks
// the matcher against `git check-ignore --no-index` on this repository's own paths).
//   - one pattern per line; blank lines and lines starting with '#' are ignored;
//   - characters A-Z a-z 0-9 . _ - + @ / * ? only: no spaces, no '!' (negation), no '\', no
//     '[...]' classes, no CR;
//   - a trailing '/' matches directories only (and submodules, which are directories on disk);
//   - a pattern with a '/' at its start or in its middle is anchored at the repository root
//     ('/docs/' = only the top-level docs directory); a pattern without one matches a file or
//     directory NAME at any depth ('docs/' = every directory called docs, '*.md' = every .md);
//   - '*' matches within one path segment, '?' one character other than '/';
//   - '**' only as a whole segment of an anchored pattern: '**/x', 'a/**/b', 'a/**';
//   - a file is ignored when its path or any of its parent directories matches; with no
//     negations there is no way to re-include it.
// Without a .workerignore in the commit the built-in DEFAULT_IGNORE applies (conservative: the
// directories no build or box script reads, INFERRED from a grep for include_str!/include_bytes!
// and script references on 2026-11-05; traces/ stays relevant because kf-abi includes a TSV from it).

export const TREE_KEY_VERSION = 'kf-tree-v1';
export const COMMIT_KEY_VERSION = 'kf-commit-v1';
export const DEFAULT_IGNORE = Object.freeze(['/archive/', '/docs/', '/site/']);
export const IGNORE_FILE = '.workerignore';
export const LIMITS = Object.freeze({
  ignoreBytes: 65536,
  patterns: 512,
  patternLength: 200,
  responseBytes: 8 * 1024 * 1024,
  totalBytes: 32 * 1024 * 1024,
  entries: 200000,
  requests: 48,
  pathBytes: 4096,
});

const ALLOWED = /^[A-Za-z0-9._+@/*?-]+$/;
const escape = (text) => text.replace(/[.+]/g, (c) => '\\' + c);

function segmentRegex(segment) {
  let out = '';
  for (const c of segment) out += c === '*' ? '[^/]*' : c === '?' ? '[^/]' : escape(c);
  return out;
}

// One pattern line -> {regex, anchored, dirOnly} | 'skip' | null (outside the subset).
export function compilePattern(line) {
  if (line === '' || line.startsWith('#')) return 'skip';
  if (line.length > LIMITS.patternLength || !ALLOWED.test(line)) return null;
  let body = line,
    dirOnly = false;
  if (body.endsWith('/')) {
    dirOnly = true;
    body = body.slice(0, -1);
  }
  let anchored = false;
  if (body.startsWith('/')) {
    anchored = true;
    body = body.slice(1);
  }
  if (body === '' || body.startsWith('/') || body.endsWith('/') || body.includes('//')) return null;
  const segments = body.split('/');
  if (segments.length > 1) anchored = true;
  for (const s of segments) {
    if (s === '.' || s === '..') return null;
    if (s.includes('**') && s !== '**') return null;
    if (s === '**' && !anchored) return null;
  }
  if (segments.length === 1 && segments[0] === '**') return null;
  let source = '';
  segments.forEach((s, i) => {
    const last = i === segments.length - 1;
    if (s === '**') {
      // '**/' (leading or middle) = zero or more directories; trailing '/**' = everything inside.
      source += last ? '.*' : '(?:.*/)?';
    } else source += segmentRegex(s) + (last ? '' : '/');
  });
  // A trailing '/**' needs at least one character below the directory.
  if (segments.at(-1) === '**') source = source.slice(0, -2) + '.+';
  return { regex: new RegExp('^' + source + '$'), anchored, dirOnly };
}

// Text of a .workerignore -> compiled patterns, or null when any line is outside the subset.
export function parseIgnore(text) {
  if (typeof text !== 'string') return null;
  if (new TextEncoder().encode(text).length > LIMITS.ignoreBytes || text.includes('\r'))
    return null;
  const patterns = [];
  for (const line of text.split('\n')) {
    const compiled = compilePattern(line);
    if (compiled === null) return null;
    if (compiled !== 'skip') patterns.push(compiled);
    if (patterns.length > LIMITS.patterns) return null;
  }
  return patterns;
}

const DEFAULT_PATTERNS = parseIgnore(DEFAULT_IGNORE.join('\n'));

function matches(p, path) {
  return p.regex.test(p.anchored ? path : path.slice(path.lastIndexOf('/') + 1));
}

// Is the entry at `path` ignored? `directory` is true for submodules (mode 160000).
export function ignored(patterns, path, directory = false) {
  const parts = path.split('/');
  for (let i = 1; i <= parts.length; i++) {
    const candidate = parts.slice(0, i).join('/'),
      isDir = i < parts.length || directory;
    for (const p of patterns) if ((!p.dirOnly || isDir) && matches(p, candidate)) return true;
  }
  return false;
}

const encoder = new TextEncoder();
function compareBytes(a, b) {
  const x = encoder.encode(a),
    y = encoder.encode(b);
  const n = Math.min(x.length, y.length);
  for (let i = 0; i < n; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return x.length - y.length;
}

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(digest), (x) => x.toString(16).padStart(2, '0')).join('');
}

const MODES = new Set(['100644', '100755', '120000', '160000']);
const validPath = (path) =>
  typeof path === 'string' &&
  path !== '' &&
  encoder.encode(path).length <= LIMITS.pathBytes &&
  !path.startsWith('/') &&
  !path.endsWith('/') &&
  !path.includes('//') &&
  !/[\0\n]/.test(path) &&
  !path.split('/').some((s) => s === '.' || s === '..' || s === '.git');

// entries: [{path, mode, sha}] (files only). Returns the hex key, or null on malformed input.
export async function treeKeyOf(entries, patterns) {
  if (!Array.isArray(patterns)) return null;
  const kept = [],
    seen = new Set();
  for (const e of entries) {
    if (!e || !validPath(e.path) || !MODES.has(e.mode) || !/^[0-9a-f]{40}$/.test(e.sha))
      return null;
    if (seen.has(e.path)) return null;
    seen.add(e.path);
    if (!ignored(patterns, e.path, e.mode === '160000')) kept.push(e);
  }
  kept.sort((a, b) => compareBytes(a.path, b.path));
  let text = TREE_KEY_VERSION + '\n';
  for (const e of kept) text += e.path + '\0' + e.mode + '\0' + e.sha + '\n';
  return sha256Hex(encoder.encode(text));
}

// The per-commit key used when no tree key exists: it names one commit, so nothing falls through.
export async function commitKey(sha) {
  if (!/^[0-9a-f]{40}$/.test(sha)) throw TypeError('commit sha');
  return sha256Hex(encoder.encode(COMMIT_KEY_VERSION + '\n' + sha + '\n'));
}

// The ignore patterns of a commit: its .workerignore when present, else DEFAULT_IGNORE.
export function patternsFor(ignoreText) {
  return ignoreText === undefined ? DEFAULT_PATTERNS : parseIgnore(ignoreText);
}

// --- GitHub trees API (CI Worker only; the public Worker never calls GitHub) ---------------

// A bounded JSON reader over the git data API. `github` is a src/github.js Github instance
// (its fetcher, headers and configuration are reused; its own get() caps responses at 256 KiB,
// which a subtree listing exceeds).
export function treeSource(github) {
  let requests = 0,
    bytes = 0;
  return async (path) => {
    if (!/^git\/(?:commits|trees|blobs)\/[0-9a-f]{40}(?:\?recursive=1)?$/.test(path))
      throw Error('tree path');
    if (++requests > LIMITS.requests) throw Error('tree requests');
    const origin = github.cfg.githubOrigin ?? 'https://api.github.com';
    const body = await fetchBounded(
      github.fetcher,
      origin + '/repos/' + github.cfg.repo + '/' + path,
      { method: 'GET', headers: github.headers },
      LIMITS.responseBytes,
    );
    bytes += body.length;
    if (bytes > LIMITS.totalBytes) throw Error('tree bytes');
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body));
  };
}

function base64Text(content) {
  const binary = atob(String(content).replace(/\n/g, ''));
  return new TextDecoder('utf-8', { fatal: true }).decode(
    Uint8Array.from(binary, (c) => c.charCodeAt(0)),
  );
}

const fileEntry = (e) =>
  e &&
  ((e.type === 'blob' && ['100644', '100755', '120000'].includes(e.mode)) ||
    (e.type === 'commit' && e.mode === '160000'));

// {key, kind: 'tree'} for a commit, or null (fail closed). `get(path)` returns parsed JSON for
// git/commits/<sha>, git/trees/<sha>[?recursive=1] and git/blobs/<sha>.
export async function computeTreeKey(get, sha) {
  if (!/^[0-9a-f]{40}$/.test(sha)) return null;
  try {
    const commit = await get('git/commits/' + sha);
    const rootSha = commit?.tree?.sha;
    if (!/^[0-9a-f]{40}$/.test(rootSha ?? '')) return null;
    const root = await get('git/trees/' + rootSha);
    if (!root || root.truncated !== false || !Array.isArray(root.tree)) return null;
    let patterns = DEFAULT_PATTERNS;
    const ignoreEntry = root.tree.find((e) => e?.path === IGNORE_FILE);
    if (ignoreEntry) {
      if (ignoreEntry.type !== 'blob' || !['100644', '100755'].includes(ignoreEntry.mode))
        return null;
      if (!/^[0-9a-f]{40}$/.test(ignoreEntry.sha ?? '')) return null;
      const blob = await get('git/blobs/' + ignoreEntry.sha);
      if (blob?.encoding !== 'base64' || !(blob.size <= LIMITS.ignoreBytes)) return null;
      patterns = parseIgnore(base64Text(blob.content));
      if (patterns === null) return null;
    }
    const entries = [],
      subtrees = [];
    for (const e of root.tree) {
      if (typeof e?.path !== 'string' || e.path.includes('/')) return null;
      if (e.type === 'tree') {
        if (e.mode !== '040000' || !/^[0-9a-f]{40}$/.test(e.sha ?? '')) return null;
        // A wholly ignored top-level directory needs no listing: nothing below can re-include.
        if (!ignored(patterns, e.path, true)) subtrees.push(e);
      } else if (fileEntry(e)) entries.push({ path: e.path, mode: e.mode, sha: e.sha });
      else return null;
    }
    const listings = await Promise.all(
      subtrees.map((t) => get('git/trees/' + t.sha + '?recursive=1')),
    );
    for (const [i, listing] of listings.entries()) {
      if (!listing || listing.truncated !== false || !Array.isArray(listing.tree)) return null;
      for (const e of listing.tree) {
        if (typeof e?.path !== 'string') return null;
        if (e.type === 'tree') continue;
        if (!fileEntry(e)) return null;
        entries.push({ path: subtrees[i].path + '/' + e.path, mode: e.mode, sha: e.sha });
        if (entries.length > LIMITS.entries) return null;
      }
    }
    const key = await treeKeyOf(entries, patterns);
    return key === null ? null : { key, kind: 'tree' };
  } catch {
    return null;
  }
}

// The stored key of a commit: the tree key, or the per-commit key when none could be computed.
export async function resultKey(get, sha) {
  const tree = await computeTreeKey(get, sha);
  return tree ?? { key: await commitKey(sha), kind: 'commit' };
}
