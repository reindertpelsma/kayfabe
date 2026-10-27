// prettier-ignore
import {allowedWorkflowRef} from './workflow-authority.js';
import { DEFAULTS } from './defaults.js';
// prettier-ignore
import {fetchJson,refuse,numericId,hexSha} from './policy.js';
export class Github {
  constructor(fetcher, cfg, token, { cache = new Map(), now = Date.now } = {}) {
    this.fetcher = (...args) => fetcher(...args);
    this.cfg = cfg;
    this.cache = cache;
    this.now = now;
    this.headers = {
      Accept: 'application/vnd.github+json',
      'User-Agent': 'kayfabe-vast-broker',
      'X-GitHub-Api-Version': '2022-11-28',
      ...(token ? { Authorization: 'Bearer ' + token } : {}),
    };
  }
  async get(path, missing = false, lifecycle = false) {
    // Cache immutable facts and explicit lifecycle reads; admission/review/head reads stay fresh.
    const immutable =
        /^commits\/[0-9a-f]{40}$/.test(path) ||
        /^contents\/scripts\/ci\/vast_report\.py\?ref=[0-9a-f]{40}$/.test(path),
      cacheKey = (lifecycle ? 'lifecycle:' : '') + path,
      cached = this.cache.get(cacheKey),
      ttl = lifecycle ? DEFAULTS.lifecycleCacheMs : 5000;
    if ((immutable || lifecycle) && cached && this.now() - cached.at < ttl)
      return structuredClone(cached.value);
    const comparison = /^compare\/[0-9a-f]{40}\.\.\.[0-9a-f]{40}\?per_page=1&page=[12]$/.test(path);
    if (
      !/^[A-Za-z0-9_/?=&.-]+$/.test(path) ||
      (path.includes('..') && !comparison) ||
      path.startsWith('/')
    )
      refuse();
    const publicFallback = async (url, init) => {
      const response = await this.fetcher(url, init);
      if (this.headers.Authorization && [401, 403].includes(response.status)) {
        await response.body?.cancel();
        const headers = { ...this.headers };
        delete headers.Authorization;
        return this.fetcher(url, { ...init, headers });
      }
      return response;
    };
    const value = await fetchJson(
      publicFallback,
      `${this.cfg.githubOrigin ?? 'https://api.github.com'}/repos/${this.cfg.repo}/${path}`,
      { method: 'GET', headers: this.headers },
      missing,
    );
    if (immutable || lifecycle) {
      if (this.cache.size >= 32) this.cache.delete(this.cache.keys().next().value);
      this.cache.set(cacheKey, { at: this.now(), value: structuredClone(value) });
    }
    return value;
  }
  // Box code runs from the tested commit. Before any paid call, its reporter must declare the
  // Worker's box protocol (an older reporter rejects the run token and bills until boot timeout).
  async boxProtocol(sha) {
    if (!hexSha(sha)) refuse('BAD_REQUEST', 400);
    const file = await this.get('contents/' + DEFAULTS.reporterPath + '?ref=' + sha, true);
    let text = '';
    try {
      if (file?.type === 'file' && file.encoding === 'base64' && typeof file.content === 'string')
        text = new TextDecoder('utf-8', { fatal: true }).decode(
          Uint8Array.from(atob(file.content.replace(/\n/g, '')), (c) => c.charCodeAt(0)),
        );
    } catch {}
    const declared = /^BOX_PROTOCOL = "([0-9]{1,4})"$/m.exec(text)?.[1];
    if (declared !== String(DEFAULTS.boxProtocol)) refuse('NOT_ELIGIBLE', 409);
  }
  async run(id, missing = false, lifecycle = false) {
    return this.get('actions/runs/' + id, missing, lifecycle);
  }
  // One exact attempt read answers the only question the Worker asks: did the owner trigger it,
  // and which commit does it test? The OIDC sha of pull_request_target names the base commit
  // (MEASURED on public runs: run.head_sha is the PR head), so only other events bind it.
  async authorize(c, requested) {
    const cfg = this.cfg;
    if (
      c.repository !== cfg.repo ||
      !numericId(c.run_id) ||
      !numericId(c.run_attempt) ||
      !hexSha(c.sha) ||
      !allowedWorkflowRef(c, cfg, cfg.profile ?? 'check')
    )
      refuse('UNAUTHORIZED', 401, undefined, 'github:claims');
    // The latest-run resource changes across reruns and can still describe a prior attempt.
    // Bind the REST lookup to the signed attempt, then independently check every returned fact.
    const r = await this.get('actions/runs/' + c.run_id + '/attempts/' + c.run_attempt);
    const checks = [
      ['id', String(r.id) === c.run_id],
      ['run_attempt', String(r.run_attempt) === c.run_attempt],
      ['event', r.event === c.event_name],
      ['status', r.status === 'in_progress'],
      ['repository', r.repository?.full_name === cfg.repo],
      ['head_repository', r.head_repository?.full_name === cfg.repo],
      ['head_sha', hexSha(r.head_sha)],
      ['head_branch', typeof r.head_branch === 'string' && !!r.head_branch],
      ['triggering_actor', String(r.triggering_actor?.id) === cfg.owner],
      ['sha', c.event_name === 'pull_request_target' || c.sha === r.head_sha],
    ];
    for (const [field, valid] of checks)
      if (!valid) refuse('UNAUTHORIZED', 401, undefined, 'github:run:' + field);
    if (requested !== undefined && requested !== r.head_sha) refuse('BINDING_MISMATCH', 409);
    return { sha: r.head_sha, branch: r.head_branch, run: c.run_id, attempt: c.run_attempt };
  }
}
