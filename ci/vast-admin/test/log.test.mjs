import test from 'node:test';
import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';
import { jwt, jwk, clock } from '../../vast-broker/test/helpers.js';
import { createAdmin } from '../worker.js';

// FALSIFIER: Access can be bypassed on logs, log markup executes, Next loses the run/cursor,
// or an open record without a provider ID cannot be stopped.
const fixture = async ({ email = 'owner@example.com', status = 200, data, rows } = {}) => {
  const calls = [];
  const bearer = await jwt({ iss: 'https://team.cloudflareaccess.com', aud: 'admin', email });
  const worker = createAdmin({
    now: () => clock,
    fetcher: async () => Response.json({ keys: [jwk] }),
  });
  const env = {
    ACCESS_TEAM_DOMAIN: 'team.cloudflareaccess.com',
    ACCESS_AUDIENCE: 'admin',
    OWNER_EMAIL: 'owner@example.com',
    LEDGER: {
      idFromName: (name) => name,
      get: () => ({
        fetch: async (request) => {
          calls.push(request);
          const cursor = Number(new URL(request.url).searchParams.get('cursor') ?? 0);
          return Response.json(
            data ?? {
              text: '<img src=x onerror=alert(1)> & "text"',
              summary: '[box] KF_FINAL_SUMMARY_BEGIN\n<script>bad()</script>\nKF_FINAL_SUMMARY_END',
              cursor,
              next_cursor: cursor + 1,
            },
            { status },
          );
        },
      }),
    },
  };
  const call = (path, token = bearer) =>
    worker.fetch(
      new Request('https://admin.example' + path, {
        headers: token === null ? {} : { 'Cf-Access-Jwt-Assertion': token },
      }),
      env,
    );
  const dom = async () => {
    const html = await (await call('/')).text();
    const nodes = new Map();
    const node = (tag) => ({
      tag,
      children: [],
      textContent: '',
      append(child) {
        this.children.push(child);
      },
      replaceChildren(fragment) {
        this.children = fragment.children;
      },
      set innerHTML(_) {
        throw Error('Markup insertion');
      },
    });
    for (const id of ['status', 'columns', 'runs', 'stop-all']) nodes.set(id, node(id));
    const browserCalls = [];
    runInNewContext(/<script>([\s\S]*?)<\/script>/.exec(html)[1], {
      document: {
        createElement: node,
        createDocumentFragment: () => node('fragment'),
        getElementById: (id) => nodes.get(id),
      },
      fetch: async (path, init = {}) => {
        browserCalls.push({ path, init });
        return Response.json(path === '/api/list' ? { runs: rows ?? [] } : { state: 'NONE' });
      },
      confirm: () => false,
      setInterval: () => {},
    });
    await new Promise((resolve) => setImmediate(resolve));
    return { nodes, browserCalls };
  };
  return { calls, call, dom };
};

for (const path of ['/log?run=123-1', '/log?run=456-2&cursor=1']) {
  test('log page needs JWT ' + path, async () => {
    const h = await fixture();
    assert.equal((await h.call(path, null)).status, 401);
    assert.equal(h.calls.length, 0);
  });
  test('log page rejects another owner ' + path, async () => {
    const h = await fixture({ email: 'other@example.com' });
    assert.equal((await h.call(path)).status, 403);
    assert.equal(h.calls.length, 0);
  });
  test('log page escapes summary/text and proxies once ' + path, async () => {
    const h = await fixture();
    const response = await h.call(path);
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('cache-control'), 'no-store');
    const html = await response.text();
    assert.match(html, /&lt;img/);
    assert.match(html, /&lt;script&gt;bad\(\)&lt;\/script&gt;/);
    assert.doesNotMatch(html, /<img|<script>/);
    assert.ok(html.indexOf('KF_FINAL_SUMMARY_BEGIN') < html.indexOf('&lt;img'));
    assert.match(html, /Next/);
    const run = new URL('https://admin.example' + path).searchParams.get('run');
    const cursor = Number(new URL('https://admin.example' + path).searchParams.get('cursor') ?? 0);
    assert.match(html, new RegExp('/log\\?run=' + run + '&amp;cursor=' + (cursor + 1)));
    assert.equal(h.calls.length, 1);
    const proxy = new URL(h.calls[0].url);
    assert.equal(proxy.origin, 'https://internal.invalid');
    assert.equal(proxy.pathname, '/admin/log');
    assert.equal(proxy.searchParams.get('run'), run);
    assert.equal(h.calls[0].headers.has('Authorization'), false);
    assert.equal(h.calls[0].headers.has('Cf-Access-Jwt-Assertion'), false);
  });
  test('expired log renders readable fixed text ' + path, async () => {
    const h = await fixture({ status: 404, data: { text: 'log expired' } });
    const response = await h.call(path);
    assert.equal(response.status, 404);
    assert.match(await response.text(), /log expired/);
    assert.equal(h.calls.length, 1);
  });
}

for (const query of [
  '',
  '?run=../secret',
  '?run=0-1',
  '?run=123-1&run=456-1',
  '?run=123-1&cursor=-1',
  '?run=123-1&extra=1',
])
  test('log page rejects malformed selection ' + query, async () => {
    const h = await fixture();
    assert.equal((await h.call('/log' + query)).status, 400);
    assert.equal(h.calls.length, 0);
  });

for (const state of ['CREATING', 'BOOTING', 'TESTING', 'GRACE', 'STOPPED'])
  test('open run gets a Stop button before provider ID, phase=' + state, async () => {
    const h = await fixture({ rows: [{ run: '123-1', state, phase: state, ended_at: null }] });
    const { nodes, browserCalls } = await h.dom();
    const row = nodes.get('runs').children[0];
    const button = row.children.flatMap((cell) => cell.children).find((n) => n.tag === 'button');
    assert.ok(button);
    await button.onclick();
    const stops = browserCalls.filter((c) => c.path === '/api/stop');
    assert.equal(stops.length, 1);
    assert.deepEqual(JSON.parse(stops[0].init.body), { run: '123-1' });
  });

for (const ended of [clock, clock - 1000])
  test('terminal row has log link and no Stop button, ended=' + ended, async () => {
    const h = await fixture({
      rows: [
        {
          run: '123-1',
          state: 'NONE',
          phase: 'NONE',
          verdict: 'FAIL',
          instance_id: 900,
          ended_at: ended,
        },
      ],
    });
    const { nodes } = await h.dom();
    const controls = nodes.get('runs').children[0].children.flatMap((cell) => cell.children);
    assert.equal(
      controls.some((n) => n.tag === 'button'),
      false,
    );
    const link = controls.find((n) => n.tag === 'a');
    assert.ok(link);
    assert.equal(link.href, '/log?run=123-1');
  });

for (const run of ['123-1', '456-2'])
  test('active row also has log link ' + run, async () => {
    const h = await fixture({ rows: [{ run, ended_at: null, state: 'BOOTING' }] });
    const { nodes } = await h.dom();
    const link = nodes
      .get('runs')
      .children[0].children.flatMap((c) => c.children)
      .find((n) => n.tag === 'a');
    assert.ok(link);
    assert.equal(link.href, '/log?run=' + run);
  });

for (const run of ['123-1', '456-2']) {
  test('last log page has no Next or fabricated summary ' + run, async () => {
    const h = await fixture({
      data: {
        text: '[box] complete\n',
        summary: '',
        cursor: 0,
        next_cursor: null,
      },
    });
    const response = await h.call('/log?run=' + run);
    assert.equal(response.status, 200);
    const html = await response.text();
    assert.match(html, /complete/);
    assert.doesNotMatch(html, /Next|KF_FINAL_SUMMARY/);
  });
  test('log broker outage is fixed error text ' + run, async () => {
    const h = await fixture({ status: 503, data: { error: 'PRIVATE-ERROR' } });
    const response = await h.call('/log?run=' + run);
    assert.equal(response.status, 503);
    const html = await response.text();
    assert.match(html, /log unavailable/);
    assert.equal(html.includes('PRIVATE-ERROR'), false);
  });
}
