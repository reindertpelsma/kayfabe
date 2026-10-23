// prettier-ignore
import {fetchJson,refuse,numericId,hexSha,skippedFiles} from './policy.js';
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
  async get(path, missing = false) {
    // Cache only immutable commit facts. Admission/run/review/file/head reads stay fresh.
    const immutable = /^commits\/[0-9a-f]{40}$/.test(path),
      cached = this.cache.get(path);
    if (immutable && cached && this.now() - cached.at < 5000) return structuredClone(cached.value);
    if (!/^[A-Za-z0-9_/?=&.-]+$/.test(path) || path.includes('..') || path.startsWith('/'))
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
    if (immutable) {
      if (this.cache.size >= 32) this.cache.delete(this.cache.keys().next().value);
      this.cache.set(path, { at: this.now(), value: structuredClone(value) });
    }
    return value;
  }
  async run(id, missing = false) {
    return this.get('actions/runs/' + id, missing);
  }
  async pr(number) {
    return this.get('pulls/' + number);
  }
  async allSkipped(p) {
    const count = p.changed_files;
    if (!Number.isSafeInteger(count) || count <= 0 || count >= 3000) return false;
    const files = [];
    for (let page = 1; page <= Math.ceil(count / 100); page++) {
      const list = await this.get(`pulls/${p.number}/files?per_page=100&page=${page}`);
      if (!Array.isArray(list) || list.length !== Math.min(100, count - files.length)) return false;
      files.push(...list);
    }
    return skippedFiles(files, count, this.cfg.patterns);
  }
  async authorize(c, hint) {
    const cfg = this.cfg;
    if (
      c.repository !== cfg.repo ||
      !numericId(c.actor_id) ||
      !numericId(c.run_id) ||
      !numericId(c.run_attempt) ||
      !hexSha(c.sha)
    )
      refuse('UNAUTHORIZED', 401);
    if (!['pull_request', 'pull_request_target', 'workflow_dispatch'].includes(c.event_name))
      refuse('UNAUTHORIZED', 401);
    let r = await this.run(c.run_id);
    if (
      String(r.id) !== c.run_id ||
      String(r.run_attempt) !== c.run_attempt ||
      r.event !== c.event_name ||
      String(r.actor?.id) !== c.actor_id ||
      r.repository?.full_name !== cfg.repo ||
      !hexSha(r.head_sha) ||
      typeof r.head_branch !== 'string' ||
      !numericId(String(r.triggering_actor?.id))
    )
      refuse('UNAUTHORIZED', 401);
    if (
      c.triggering_actor !== undefined &&
      String(c.triggering_actor) !== String(r.triggering_actor.id)
    )
      refuse('UNAUTHORIZED', 401);
    if (cfg.profile === 'sweep') {
      if (
        c.event_name !== 'workflow_dispatch' ||
        c.actor_id !== cfg.owner ||
        String(r.triggering_actor.id) !== cfg.owner ||
        c.sha !== r.head_sha ||
        c.ref !== 'refs/heads/' + r.head_branch
      )
        refuse('UNAUTHORIZED', 401);
      const selected = hint?.tested_sha ?? c.sha;
      if (!hexSha(selected)) refuse('BAD_REQUEST', 400);
      const commit = await this.get('commits/' + selected);
      if (commit.sha !== selected) refuse();
      return {
        sha: selected,
        pr: null,
        branch: r.head_branch,
        run: c.run_id,
        attempt: c.run_attempt,
      };
    }
    let p;
    const automatic = ['pull_request', 'pull_request_target'].includes(c.event_name);
    if (c.event_name === 'pull_request_target') {
      if (
        !cfg.bases.some((base) => c.ref === 'refs/heads/' + base) ||
        c.sha !== r.head_sha ||
        c.workflow_sha !== r.head_sha ||
        !cfg.checkWorkflows.includes(c.workflow_ref) ||
        !Number.isSafeInteger(hint?.pr_number)
      )
        refuse('UNAUTHORIZED', 401);
      const member = r.pull_requests?.filter(
        (x) => x.number === hint.pr_number && x.head?.sha === hint.tested_sha,
      );
      if (member?.length !== 1) refuse('UNAUTHORIZED', 401);
      p = await this.pr(hint.pr_number);
      if (p.head?.repo?.full_name !== cfg.repo) refuse('NOT_ELIGIBLE', 403);
      if (p.head?.sha !== hint.tested_sha) refuse('UNAUTHORIZED', 401);
      r = { ...r, head_sha: hint.tested_sha, head_branch: p.head.ref };
    } else if (automatic) {
      const m = /^refs\/pull\/([1-9][0-9]*)\/merge$/.exec(c.ref || '');
      if (!m) refuse('UNAUTHORIZED', 401);
      const member = r.pull_requests?.filter(
        (x) => String(x.number) === m[1] && x.head?.sha === r.head_sha,
      );
      if (member?.length !== 1) refuse('UNAUTHORIZED', 401);
      p = await this.pr(m[1]);
    } else {
      if (
        c.actor_id !== cfg.owner ||
        String(r.triggering_actor.id) !== cfg.owner ||
        c.sha !== r.head_sha ||
        c.ref !== 'refs/heads/' + r.head_branch
      )
        refuse('UNAUTHORIZED', 401);
      const commit = await this.get('commits/' + c.sha);
      if (commit.sha !== c.sha) refuse();
      const prs = await this.get('commits/' + c.sha + '/pulls?per_page=100');
      if (!Array.isArray(prs) || prs.length >= 100) refuse();
      const exact = prs.filter(
        (x) =>
          x.state === 'open' &&
          x.head?.sha === c.sha &&
          x.head?.ref === r.head_branch &&
          x.head?.repo?.full_name === cfg.repo,
      );
      if (exact.length > 1) refuse();
      p = exact[0];
      if (!p)
        return {
          sha: r.head_sha,
          pr: null,
          branch: r.head_branch,
          run: c.run_id,
          attempt: c.run_attempt,
        };
    }
    if (
      !p ||
      p.state !== 'open' ||
      !Number.isSafeInteger(p.number) ||
      p.number <= 0 ||
      p.head?.sha !== r.head_sha ||
      p.head?.ref !== r.head_branch ||
      p.head?.repo?.full_name !== cfg.repo ||
      p.base?.repo?.full_name !== cfg.repo
    )
      refuse();
    if (automatic && !cfg.bases.includes(p.base.ref)) refuse('NOT_ELIGIBLE', 403); // MUTANT base
    const target = { sha: p.head.sha, pr: p.number, branch: r.head_branch };
    if (p.draft === true && !cfg.drafts) refuse('NEEDS_APPROVAL', 403);
    let reviews = [];
    for (let page = 1; page <= 5; page++) {
      const list = await this.get(`pulls/${p.number}/reviews?per_page=100&page=${page}`);
      if (!Array.isArray(list)) refuse();
      reviews.push(...list);
      if (list.length < 100) break;
      if (page === 5) refuse();
    }
    const own = reviews
      .filter((x) => String(x.user?.id) === cfg.owner && x.state !== 'PENDING')
      .sort((a, b) => a.id - b.id);
    const decisive = own.filter((x) =>
      ['APPROVED', 'DISMISSED', 'CHANGES_REQUESTED'].includes(x.state),
    );
    const latest = decisive.at(-1);
    const blocked = latest && ['DISMISSED', 'CHANGES_REQUESTED'].includes(latest.state);
    const approved = latest?.state === 'APPROVED' && latest.commit_id === p.head.sha;
    const ownerPush = c.actor_id === cfg.owner && String(r.triggering_actor.id) === cfg.owner; // MUTANT owner
    const author = String(p.user?.id) === cfg.owner;
    const permitted =
      cfg.mode === 'actor_only'
        ? ownerPush
        : !blocked &&
          (cfg.mode === 'owner_push'
            ? ownerPush
            : cfg.mode === 'owner_push_or_review'
              ? ownerPush || approved
              : cfg.mode === 'review_or_author'
                ? author || approved
                : approved);
    if (!permitted) refuse('NEEDS_APPROVAL', 403);
    const notNeeded = automatic && (await this.allSkipped(p));
    const current = await this.pr(p.number);
    if (current.head?.sha !== target.sha || current.state !== 'open') refuse();
    if (automatic && !cfg.bases.includes(current.base?.ref)) refuse('NOT_ELIGIBLE', 403);
    return { ...target, run: c.run_id, attempt: c.run_attempt, notNeeded };
  }
}
