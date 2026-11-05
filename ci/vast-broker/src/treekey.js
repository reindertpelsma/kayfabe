import { fetchBounded } from './policy.js';
// The result key of a commit: a hash of its tree after the commit's own .workerignore.
// Design: docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md §3 (AGREED RECOMMENDATION, 2026-11-05).
//
// key = SHA-256 over 'kf-tree-v2\n' + 'ignore ' + SHA-256(effective pattern lines) + '\n' + for
//       every unignored file, sorted by the UTF-8 bytes of its path: path NUL mode NUL object-sha
//       LF. Files are blobs (100644, 100755), symlinks (120000) and submodules (160000);
//       directories are not entries. The effective pattern lines are the non-blank, non-comment
//       lines of the ignore set, so two different ignore sets never share a key.
// Commits with identical filtered trees and ignore sets share a key. A commit that newly ignores
// a file removes it from the list, so its key changes; a new file added together with its
// exclusion never enters the list. CAUTION (review of #258, H2): until the box deletes ignored
// files after cloning (not built), a key does not prove what a run tested, so results.js counts
// rows of the same commit only unless TREE_KEY_CARRY_OVER is 'true'.
//
// FAIL CLOSED: any pattern outside the subset below, an ignore set that covers a code root
// (PROTECTED), a truncated, oversized or empty tree listing, a filtered tree with no file, an
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
//   - '*' matches within one path segment, '?' one byte other than '/' (git matches bytes);
//   - '**' only as a whole segment of an anchored pattern: '**/x', 'a/**/b', 'a/**';
//   - a file is ignored when its path or any of its parent directories matches; with no
//     negations there is no way to re-include it.
// Matching uses no regular expression: a two-pointer glob per segment and a table over segments,
// so the cost is bounded by pattern length x path length (review of #258, H1: a backtracking
// RegExp took 10 s for '*a*a*a*a*a*b' against 100 'a's).
// Without a .workerignore in the commit the built-in DEFAULT_IGNORE applies (conservative: the
// directories no build or box script reads, INFERRED from a grep for include_str!/include_bytes!
// and script references on 2026-11-05; traces/ stays relevant because kf-abi includes a TSV from it).

export const TREE_KEY_VERSION = 'kf-tree-v2';
export const COMMIT_KEY_VERSION = 'kf-commit-v1';
export const DEFAULT_IGNORE = Object.freeze(['/archive/', '/docs/', '/site/']);
export const IGNORE_FILE = '.workerignore';
// Code roots an ignore set may never cover, as [path, isDirectory]. A pattern that ignores one of
// them (or '*', which ignores all) makes the whole ignore set unusable: no tree key.
export const PROTECTED = Object.freeze([
  ['Cargo.toml', false],
  ['Cargo.lock', false],
  ['rust-toolchain.toml', false],
  ['.cargo', true],
  ['crates', true],
  ['scripts', true],
  ['ci', true],
  ['ci/vast-broker', true],
  ['qemu', true],
  ['cuda', true],
]);
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
const encoder = new TextEncoder();
const STAR = 42,
  QUESTION = 63;
const ANY = Symbol('**');

// One segment against one glob segment, on UTF-8 bytes. Two pointers with a single backtrack
// point: at most len(pattern) x len(text) steps, never exponential.
export function globSegment(pattern, text) {
  let i = 0,
    j = 0,
    star = -1,
    mark = 0;
  while (j < text.length) {
    if (i < pattern.length && pattern[i] === STAR) {
      star = i++;
      mark = j;
    } else if (i < pattern.length && (pattern[i] === QUESTION || pattern[i] === text[j])) {
      i++;
      j++;
    } else if (star >= 0) {
      i = star + 1;
      j = ++mark;
    } else return false;
  }
  while (i < pattern.length && pattern[i] === STAR) i++;
  return i === pattern.length;
}

// Pattern segments against the first n path segments. '**' (ANY) matches zero or more segments,
// a trailing '**' one or more. A table over (pattern segment, path segment).
function matchSegments(segments, parts, n) {
  const P = segments.length;
  let next = new Array(n + 1).fill(false);
  next[n] = true;
  for (let i = P - 1; i >= 0; i--) {
    const row = new Array(n + 1).fill(false);
    for (let j = n; j >= 0; j--) {
      if (segments[i] === ANY) row[j] = i === P - 1 ? j < n : next[j] || (j < n && row[j + 1]);
      else row[j] = j < n && next[j + 1] && globSegment(segments[i], parts[j]);
    }
    next = row;
  }
  return next[0];
}

// One pattern line -> {segments, anchored, dirOnly, line} | 'skip' | null (outside the subset).
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
  const parts = body.split('/');
  if (parts.length > 1) anchored = true;
  for (const s of parts) {
    if (s === '.' || s === '..') return null;
    if (s.includes('**') && s !== '**') return null;
    if (s === '**' && !anchored) return null;
  }
  if (parts.length === 1 && parts[0] === '**') return null;
  const segments = parts.map((s) => (s === '**' ? ANY : encoder.encode(s)));
  return { segments, anchored, dirOnly, line };
}

// Text of a .workerignore -> compiled patterns (with .lines, the effective lines), or null when
// any line is outside the subset.
export function parseIgnore(text) {
  if (typeof text !== 'string') return null;
  if (encoder.encode(text).length > LIMITS.ignoreBytes || text.includes('\r')) return null;
  const patterns = [];
  for (const line of text.split('\n')) {
    const compiled = compilePattern(line);
    if (compiled === null) return null;
    if (compiled !== 'skip') patterns.push(compiled);
    if (patterns.length > LIMITS.patterns) return null;
  }
  patterns.lines = patterns.map((p) => p.line);
  return patterns;
}

// Is the entry at `path` ignored? `directory` is true for submodules (mode 160000).
export function ignored(patterns, path, directory = false) {
  const parts = path.split('/').map((s) => encoder.encode(s));
  for (let n = 1; n <= parts.length; n++) {
    const isDir = n < parts.length || directory;
    for (const p of patterns) {
      if (p.dirOnly && !isDir) continue;
      if (
        p.anchored ? matchSegments(p.segments, parts, n) : globSegment(p.segments[0], parts[n - 1])
      )
        return true;
    }
  }
  return false;
}

// An ignore set that covers a code root is refused (null).
export function guarded(patterns) {
  if (!Array.isArray(patterns)) return null;
  for (const [path, dir] of PROTECTED) if (ignored(patterns, path, dir)) return null;
  return patterns;
}

const DEFAULT_PATTERNS = guarded(parseIgnore(DEFAULT_IGNORE.join('\n')));

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
  !/[\0\n\ufffd]/.test(path) &&
  !path.split('/').some((s) => s === '.' || s === '..' || s === '.git');

// entries: [{path, mode, sha}] (files only). Returns the hex key, or null on malformed input or
// when no file survives the ignore set.
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
  if (!kept.length) return null;
  kept.sort((a, b) => compareBytes(a.path, b.path));
  const ignoreDigest = await sha256Hex(encoder.encode((patterns.lines ?? []).join('\n')));
  let text = TREE_KEY_VERSION + '\nignore ' + ignoreDigest + '\n';
  for (const e of kept) text += e.path + '\0' + e.mode + '\0' + e.sha + '\n';
  return sha256Hex(encoder.encode(text));
}

// The per-commit key used when no tree key exists: it names one commit, so nothing falls through.
export async function commitKey(sha) {
  if (!/^[0-9a-f]{40}$/.test(sha)) throw TypeError('commit sha');
  return sha256Hex(encoder.encode(COMMIT_KEY_VERSION + '\n' + sha + '\n'));
}

// The ignore patterns of a commit: its .workerignore when present, else DEFAULT_IGNORE; null when
// the file is outside the subset or covers a code root.
export function patternsFor(ignoreText) {
  return ignoreText === undefined ? DEFAULT_PATTERNS : guarded(parseIgnore(ignoreText));
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
    if (!root || root.truncated !== false || !Array.isArray(root.tree) || !root.tree.length)
      return null;
    let patterns = DEFAULT_PATTERNS;
    const ignoreEntry = root.tree.find((e) => e?.path === IGNORE_FILE);
    if (ignoreEntry) {
      if (ignoreEntry.type !== 'blob' || !['100644', '100755'].includes(ignoreEntry.mode))
        return null;
      if (!/^[0-9a-f]{40}$/.test(ignoreEntry.sha ?? '')) return null;
      const blob = await get('git/blobs/' + ignoreEntry.sha);
      if (blob?.encoding !== 'base64' || !(blob.size <= LIMITS.ignoreBytes)) return null;
      patterns = patternsFor(base64Text(blob.content));
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
