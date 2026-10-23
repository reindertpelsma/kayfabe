// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {readBounded,refuse} from './policy.js';
// Resolve metadata only. Never follow a redirect or acquire release bytes.
export async function checkBlobBudget(env, ledger, record, tag, name, now) {
  const safe = (s, n) =>
    typeof s === 'string' &&
    new RegExp('^[A-Za-z0-9._-]{1,' + n + '}$').test(s) &&
    s !== '.' &&
    s !== '..';
  if (!safe(tag, 64) || !safe(name, 100)) refuse('BAD_REQUEST', 400);
  if (
    !/^[-\w.]+\/[-\w.]+$/.test(env.BLOB_REPO ?? '') ||
    typeof (env.GITHUB_API_TOKEN || env.BLOB_GITHUB_TOKEN) !== 'string' ||
    !(env.GITHUB_API_TOKEN || env.BLOB_GITHUB_TOKEN) ||
    /[\r\n]/.test(env.GITHUB_API_TOKEN || env.BLOB_GITHUB_TOKEN)
  )
    refuse();
  let prefixes;
  try {
    prefixes = JSON.parse(env.BLOB_ALLOWED_TAG_PREFIXES ?? '[]');
  } catch {
    refuse();
  }
  if (
    !Array.isArray(prefixes) ||
    !prefixes.length ||
    prefixes.length > 32 ||
    !prefixes.every((x) => safe(x, 64))
  )
    refuse();
  if (prefixes.length && !prefixes.some((p) => tag.startsWith(p))) refuse('UNAUTHORIZED', 401);
  const bound = (key, fallback, max) => {
    const v = env[key] ?? String(fallback);
    if (!/^[1-9][0-9]{0,3}$/.test(v) || Number(v) > max) refuse();
    return Number(v);
  };
  const runLimit = bound('BLOB_RUN_MINTS', DEFAULTS.blobRunMints, 128),
    dayLimit = bound('BLOB_DAILY_MINTS', DEFAULTS.blobDailyMints, 4096),
    dayKey = 'blob-day:' + Math.floor(now / 86400000),
    used = (await ledger.get(dayKey)) ?? 0;
  if ((record.blobMints ?? 0) >= runLimit || used >= dayLimit) refuse('RATE_LIMITED', 429);
  return { dayKey, used };
}
export async function mintBlob(fetcher, env, ledger, record, tag, name, now) {
  const { dayKey, used } = await checkBlobBudget(env, ledger, record, tag, name, now);
  record.blobMints = (record.blobMints ?? 0) + 1;
  await ledger.save(record);
  await ledger.put(dayKey, used + 1);
  const headers = {
      Accept: 'application/vnd.github+json',
      Authorization: 'Bearer ' + (env.GITHUB_API_TOKEN || env.BLOB_GITHUB_TOKEN),
      'User-Agent': 'kayfabe-vast-broker',
      'X-GitHub-Api-Version': '2022-11-28',
    },
    base = 'https://api.github.com/repos/' + env.BLOB_REPO + '/releases/';
  const metadata = await fetcher(base + 'tags/' + tag, {
    method: 'GET',
    headers,
    redirect: 'manual',
    signal: AbortSignal.timeout(8000),
  });
  if (metadata.status !== 200 || metadata.redirected) {
    await metadata.body?.cancel();
    refuse();
  }
  const bytes = await readBounded(metadata, 65536);
  let release;
  try {
    release = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    refuse();
  }
  if (!release || !Array.isArray(release.assets) || release.assets.length > 100) refuse();
  const assets = release.assets.filter((x) => x?.name === name);
  if (assets.length !== 1) refuse();
  const asset = assets[0];
  if (
    !Number.isSafeInteger(asset.id) ||
    asset.id <= 0 ||
    !Number.isSafeInteger(asset.size) ||
    asset.size < 0 ||
    asset.size > 1000000000000
  )
    refuse();
  const response = await fetcher(base + 'assets/' + asset.id, {
    method: 'GET',
    headers: { ...headers, Accept: 'application/octet-stream' },
    redirect: 'manual',
    signal: AbortSignal.timeout(8000),
  });
  try {
    if (response.status !== 302 || response.redirected) refuse();
    const location = response.headers.get('Location');
    if (!location || location.length > 8192) refuse();
    let decoded;
    try {
      decoded = decodeURIComponent(location);
    } catch {
      refuse();
    }
    if (
      [env.GITHUB_API_TOKEN, env.BLOB_GITHUB_TOKEN].some(
        (value) => value && (location.includes(value) || decoded.includes(value)),
      )
    )
      refuse();
    let url;
    try {
      url = new URL(location);
    } catch {
      refuse();
    }
    if (
      url.protocol !== 'https:' ||
      url.username ||
      url.password ||
      url.port ||
      url.hash ||
      ![
        'release-assets.githubusercontent.com',
        'objects.githubusercontent.com',
        'github-releases.githubusercontent.com',
      ].includes(url.hostname)
    )
      refuse();
    return { url: location, size: asset.size, name };
  } finally {
    await response.body?.cancel();
  }
}
