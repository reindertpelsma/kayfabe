import { createRemoteJWKSet, customFetch, jwtVerify } from 'jose';

export const createAdmin = ({ fetcher = fetch, now = Date.now } = {}) => ({
  async fetch(request, env) {
    const headers = {
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
      'Referrer-Policy': 'no-referrer',
      'Content-Security-Policy':
        "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; " +
        "base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
    };
    const json = (data, status = 200) => Response.json(data, { status, headers });
    if (
      !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.cloudflareaccess\.com$/.test(
        env.ACCESS_TEAM_DOMAIN ?? '',
      ) ||
      typeof env.ACCESS_AUDIENCE !== 'string' ||
      !env.ACCESS_AUDIENCE.trim() ||
      typeof env.OWNER_EMAIL !== 'string' ||
      !/^[^\s@]+@[^\s@]+$/.test(env.OWNER_EMAIL)
    )
      return json({ error: 'Access policy unconfigured' }, 503);
    const assertion = request.headers.get('Cf-Access-Jwt-Assertion');
    if (!assertion || assertion.length > 8192) return json({ error: 'Unauthorized' }, 401);
    let identity;
    try {
      // All verification objects and I/O are request-local, including the JWKS cache.
      const issuer = 'https://' + env.ACCESS_TEAM_DOMAIN;
      const keys = createRemoteJWKSet(new URL(issuer + '/cdn-cgi/access/certs'), {
        [customFetch]: fetcher,
      });
      const verified = await jwtVerify(assertion, keys, {
        algorithms: ['RS256'],
        issuer,
        audience: env.ACCESS_AUDIENCE,
        currentDate: new Date(now()),
        requiredClaims: ['exp', 'iat'],
      });
      identity = verified.payload;
    } catch {
      return json({ error: 'Unauthorized' }, 401);
    }
    if (identity.email !== env.OWNER_EMAIL) return json({ error: 'Forbidden' }, 403);
    const url = new URL(request.url);
    if (url.pathname === '/log' && request.method === 'GET') {
      const params = url.searchParams,
        run = params.get('run'),
        cursor = params.get('cursor') ?? '0';
      if (
        params.getAll('run').length !== 1 ||
        params.getAll('cursor').length > 1 ||
        [...params.keys()].some((key) => !['run', 'cursor'].includes(key)) ||
        !/^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(run ?? '') ||
        !/^(0|[1-9][0-9]{0,6})$/.test(cursor)
      )
        return json({ error: 'Invalid log selection' }, 400);
      const escape = (text) =>
        String(text).replace(
          /[&<>"']/g,
          (c) =>
            ({
              '&': '&amp;',
              '<': '&lt;',
              '>': '&gt;',
              '"': '&quot;',
              "'": '&#39;',
            })[c],
        );
      const html = (body, status = 200) =>
        new Response('<!doctype html><meta charset="utf-8"><title>Run log</title>' + body, {
          status,
          headers: { ...headers, 'Content-Type': 'text/html; charset=utf-8' },
        });
      try {
        const stub = env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account-enam-v1'));
        const response = await stub.fetch(
          new Request('https://internal.invalid/admin/log?run=' + run + '&cursor=' + cursor),
        );
        if (response.status === 404) return html('<pre>log expired</pre>', 404);
        if (!response.ok) return html('<pre>log unavailable</pre>', response.status);
        const page = await response.json();
        const summary = page.summary ? '<pre>' + escape(page.summary) + '</pre>' : '';
        const next =
          Number.isSafeInteger(page.next_cursor) && page.next_cursor > Number(cursor)
            ? '<a href="' + escape('/log?run=' + run + '&cursor=' + page.next_cursor) + '">Next</a>'
            : '';
        return html(summary + '<pre>' + escape(page.text) + '</pre>' + next);
      } catch {
        return html('<pre>log unavailable</pre>', 503);
      }
    }
    if (url.search) return json({ error: 'Not found' }, 404);
    if (url.pathname === '/' && request.method === 'GET')
      return new Response(PAGE, {
        headers: { ...headers, 'Content-Type': 'text/html; charset=utf-8' },
      });
    const list = url.pathname === '/api/list' && request.method === 'GET';
    const stop = url.pathname === '/api/stop' && request.method === 'POST';
    // 2026-11-05 (wtier): plan (rents nothing) and start (the same admission path as a
    // PR-triggered rental) of a small, middle or large (release) run, owner only like stop.
    const start = url.pathname === '/api/start' && request.method === 'POST';
    const plan = url.pathname === '/api/plan' && request.method === 'POST';
    if (!list && !stop && !start && !plan) return json({ error: 'Not found' }, 404);
    let body;
    if (start || plan) {
      if (request.headers.has('Origin') && request.headers.get('Origin') !== url.origin)
        return json({ error: 'Forbidden' }, 403);
      if (request.headers.get('Content-Type')?.split(';')[0].trim() !== 'application/json')
        return json({ error: 'Invalid request' }, 400);
      try {
        const reader = request.body?.getReader();
        if (!reader) throw Error();
        const chunks = [];
        let size = 0;
        try {
          while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            size += value.byteLength;
            if (size > 512) throw Error();
            chunks.push(value);
          }
        } finally {
          await reader.cancel().catch(() => {});
        }
        const run = JSON.parse(await new Blob(chunks).text());
        if (
          !run ||
          typeof run !== 'object' ||
          Array.isArray(run) ||
          !Object.keys(run).every((k) => ['commit_sha', 'tier', 'die'].includes(k)) ||
          typeof run.commit_sha !== 'string' ||
          !/^[0-9a-f]{40}$/.test(run.commit_sha) ||
          !['small', 'middle', 'large'].includes(run.tier) ||
          (run.die !== undefined &&
            (typeof run.die !== 'string' || !/^[A-Z][A-Z0-9_]{1,23}$/.test(run.die)))
        )
          throw Error();
        body = JSON.stringify(run);
      } catch {
        return json({ error: 'Invalid request' }, 400);
      }
      try {
        const stub = env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account-enam-v1'));
        const response = await stub.fetch(
          new Request('https://internal.invalid/admin/' + (start ? 'start' : 'plan'), {
            method: 'POST',
            body,
            headers: { 'Content-Type': 'application/json' },
          }),
        );
        // The DO answers with a closed shape (plan, batch view or a refusal code); relay it.
        const answer = await response.json();
        return json(answer, response.status);
      } catch {
        return json({ error: 'Broker unavailable' }, 503);
      }
    }
    if (stop) {
      if (request.headers.has('Origin') && request.headers.get('Origin') !== url.origin)
        return json({ error: 'Forbidden' }, 403);
      if (request.headers.get('Content-Type')?.split(';')[0].trim() !== 'application/json')
        return json({ error: 'Invalid selection' }, 400);
      try {
        const reader = request.body?.getReader();
        if (!reader) throw Error();
        const chunks = [];
        let size = 0;
        try {
          while (true) {
            const { value, done } = await reader.read();
            if (done) break;
            size += value.byteLength;
            if (size > 256) throw Error();
            chunks.push(value);
          }
        } finally {
          await reader.cancel().catch(() => {});
        }
        const selection = JSON.parse(await new Blob(chunks).text());
        if (
          !selection ||
          Object.keys(selection).length !== 1 ||
          !(
            selection.all === true ||
            (typeof selection.run === 'string' &&
              /^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(selection.run)) ||
            (typeof selection.batch === 'string' && /^8[0-9]{18}$/.test(selection.batch))
          )
        )
          throw Error();
        body = JSON.stringify(selection);
      } catch {
        return json({ error: 'Invalid selection' }, 400);
      }
    }
    try {
      const stub = env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account-enam-v1'));
      const response = await stub.fetch(
        new Request('https://internal.invalid/admin/' + (list ? 'list' : 'stop'), {
          method: list ? 'GET' : 'POST',
          ...(stop ? { body, headers: { 'Content-Type': 'application/json' } } : {}),
        }),
      );
      if (!response.ok) return json({ error: 'Broker unavailable' }, response.status);
      return json(await response.json());
    } catch {
      return json({ error: 'Broker unavailable' }, 503);
    }
  },
});

const PAGE = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>Vast CI runs</title>
<button id="stop-all" type="button">Stop all</button>
<p id="status" role="status"></p>
<fieldset>
  <legend>Start a run (plan first; a release is tier large)</legend>
  <label>Commit <input id="start-sha" size="42" maxlength="40" pattern="[0-9a-f]{40}"></label>
  <label>Tier <select id="start-tier">
    <option value="middle">middle</option>
    <option value="small">small</option>
    <option value="large">large (release)</option>
  </select></label>
  <label>Die (optional, not for large) <input id="start-die" size="8" maxlength="24"></label>
  <button id="plan" type="button">Plan</button>
  <button id="start" type="button">Start</button>
</fieldset>
<pre id="start-out"></pre>
<pre id="batches"></pre>
<table>
  <thead><tr id="columns"></tr></thead>
  <tbody id="runs"></tbody>
</table>
<script>
const fields = [
  'run', 'sha', 'branch', 'profile', 'tier', 'family', 'die', 'gpu_model', 'machine_id', 'host_id',
  'instance_id', 'state', 'phase', 'verdict', 'cost_estimate_usd',
  'started_at', 'ended_at', 'boot_seconds'
];
const status = document.getElementById('status');
for (const field of [...fields, 'Log', 'Stop']) {
  const th = document.createElement('th');
  th.textContent = field;
  document.getElementById('columns').append(th);
}
async function stop(selection) {
  try {
    const response = await fetch('/api/stop', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(selection)
    });
    if (!response.ok) throw Error('Stop failed (' + response.status + ')');
    await refresh();
  } catch (error) { status.textContent = error.message; }
}
function runRequest() {
  const body = {
    commit_sha: document.getElementById('start-sha').value.trim(),
    tier: document.getElementById('start-tier').value
  };
  const die = document.getElementById('start-die').value.trim();
  if (die) body.die = die;
  return body;
}
async function post(path, body) {
  const out = document.getElementById('start-out');
  try {
    const response = await fetch(path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    });
    out.textContent = JSON.stringify(await response.json(), null, 2);
  } catch (error) { out.textContent = error.message; }
}
const planButton = document.getElementById('plan');
if (planButton) planButton.onclick = () => post('/api/plan', runRequest());
const startButton = document.getElementById('start');
if (startButton) startButton.onclick = async () => {
  const body = runRequest();
  if (!confirm('Start a ' + body.tier + ' run of ' + body.commit_sha + '? It rents boxes.')) return;
  await post('/api/start', body);
  await refresh();
};
async function refresh() {
  try {
    const response = await fetch('/api/list');
    if (!response.ok) throw Error('Refresh failed (' + response.status + ')');
    const data = await response.json();
    const batches = document.getElementById('batches');
    if (batches) batches.textContent = (data.batches || []).map((b) =>
      [b.batch, b.tier, b.sha, b.state, 'verdict=' + b.verdict, 'legs=' + b.legs.length,
       'est=' + b.estimate_usd, 'spent=' + b.spent_usd].join(' ')).join('\\n');
    const rows = document.createDocumentFragment();
    for (const run of data.runs) {
      const tr = document.createElement('tr');
      for (const field of fields) {
        const td = document.createElement('td');
        const value = run[field];
        td.textContent = value == null ? '' : field.endsWith('_at')
          ? new Date(value).toISOString() : String(value);
        tr.append(td);
      }
      const log = document.createElement('td');
      const link = document.createElement('a');
      link.textContent = 'log';
      link.href = '/log?run=' + encodeURIComponent(run.run);
      log.append(link);
      tr.append(log);
      const td = document.createElement('td');
      if (run.ended_at === null) {
        const button = document.createElement('button');
        button.textContent = 'Stop';
        button.onclick = async () => {
          button.disabled = true;
          await stop({ run: run.run });
          button.disabled = false;
        };
        td.append(button);
      }
      tr.append(td);
      rows.append(tr);
    }
    document.getElementById('runs').replaceChildren(rows);
    status.textContent = '';
  } catch (error) { status.textContent = error.message; }
}
document.getElementById('stop-all').onclick = async (event) => {
  if (!confirm('Stop all active runs?')) return;
  event.target.disabled = true;
  await stop({ all: true });
  event.target.disabled = false;
};
refresh();
setInterval(refresh, 10000);
</script>
</html>`;

export default createAdmin();
