// The /status/ section: platform coverage pages (docs/design/V3_PLATFORM_TESTING.md section 9).
// No dependencies and no URL literals: the API origin, the log origin and the repository come
// from <meta> tags written by site/status_pages.py. Every value from the API is inserted with
// textContent or as an encoded query parameter, never as HTML. No GitHub API call is made;
// commit links are plain hyperlinks.
(() => {
  'use strict';
  const script = document.currentScript;
  const meta = (name) => document.querySelector('meta[name="' + name + '"]')?.content ?? '';
  const mode = meta('kf-status-mode');
  const api = meta('kf-status-api');
  const logs = meta('kf-status-logs');
  const repo = meta('kf-status-repo');
  const root = new URL('../', script.src);
  const fileMode = location.protocol === 'file:';
  const main = document.getElementById('main');
  const state = document.getElementById('kf-status-state');
  const content = document.getElementById('kf-status-content');
  const filters = document.getElementById('kf-status-filters');
  if (!main || !state || !content || (mode !== 'live' && mode !== 'fixture')) return;

  const PATTERN = {
    die: /^[A-Z]{2}[0-9]{3}[A-Z]?$/,
    suite: /^[a-z]+-[a-z-]{2,30}$/,
    id: /^[a-z0-9][a-z0-9-]{5,63}$/,
    sha: /^[0-9a-f]{40}$/,
    cursor: /^[A-Za-z0-9_-]{8,200}$/,
    family: /^[a-z][a-z0-9]{2,15}$/,
    driver: /^[0-9]{3}\.[0-9]{1,3}(?:\.[0-9]{1,3})?$/,
    cpu: /^(?:intel|amd)$/,
    nested: /^[01]$/,
  };

  // --- data -----------------------------------------------------------------------------------
  let fixture = null;
  async function load(path) {
    if (mode === 'fixture') {
      if (!fixture) {
        const response = await fetch(new URL(meta('kf-status-fixture'), script.src));
        if (!response.ok) throw new Error('The fixture file could not be read.');
        fixture = (await response.json()).responses ?? {};
      }
      if (!Object.hasOwn(fixture, path))
        throw new Error('This view is not in the synthetic fixture.');
      return fixture[path];
    }
    const response = await fetch(api + path, {
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      headers: { Accept: 'application/json' },
    });
    if (response.status === 404) throw new Error('Not found.');
    if (!response.ok) throw new Error('The status service answered ' + response.status + '.');
    return response.json();
  }

  // --- DOM helpers ----------------------------------------------------------------------------
  function el(tag, attrs, ...children) {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(attrs ?? {})) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') node.className = value;
      else node.setAttribute(key, value === true ? '' : String(value));
    }
    for (const child of children.flat(Infinity)) {
      if (child === null || child === undefined || child === false) continue;
      node.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return node;
  }
  const page = (name, params) => {
    const url = new URL(
      'status/' + (name ? name + '/' : '') + (fileMode ? 'index.html' : ''),
      root,
    );
    for (const [key, value] of Object.entries(params ?? {}))
      if (value !== null && value !== undefined && value !== '') url.searchParams.set(key, value);
    return url.href;
  };
  const short = (sha) => (typeof sha === 'string' ? sha.slice(0, 10) : '');
  const commitLink = (sha) =>
    PATTERN.sha.test(sha ?? '')
      ? el('a', { href: repo + '/commit/' + sha, class: 'kf-sha' }, short(sha))
      : el('span', {}, 'unknown commit');
  const when = (iso) =>
    typeof iso === 'string' ? iso.replace('T', ' ').replace(/:00Z$/, 'Z') : '';
  const VERDICTS = new Set(['PASS', 'FAIL', 'RETRY', 'UNAVAILABLE']);
  function verdict(value, running) {
    const word = running ? 'running' : VERDICTS.has(value) ? value : 'no data';
    const key = running ? 'running' : VERDICTS.has(value) ? value.toLowerCase() : 'none';
    return el('span', { class: 'kf-v kf-v-' + key }, word);
  }
  function fail(error) {
    state.textContent = error instanceof Error ? error.message : 'Loading failed.';
    state.classList.add('kf-error');
  }
  function done(text) {
    state.textContent = text;
  }
  const query = new URLSearchParams(location.search);
  const param = (name) => {
    const value = query.get(name);
    return value !== null && PATTERN[name === 'before' ? 'cursor' : name].test(value)
      ? value
      : null;
  };

  function scoreText(run) {
    if (run.scores_hidden) return 'not published (' + run.scores_hidden + ')';
    const entries = Object.entries(run.scores ?? {});
    if (!entries.length) return '';
    return entries.map(([name, ratio]) => name + ' ' + Number(ratio).toFixed(2) + 'x').join(', ');
  }
  function platformText(p) {
    if (!p) return '';
    return [
      'host ' + p.host_driver,
      p.cpu_vendor,
      p.nested ? 'nested' : 'bare metal',
      p.guest_os + ' guest ' + p.guest_driver,
    ].join(', ');
  }

  // --- matrix ---------------------------------------------------------------------------------
  async function matrix() {
    const chosen = {};
    for (const name of ['family', 'driver', 'cpu', 'nested']) chosen[name] = param(name);
    chosen.commit =
      query.get('commit') && PATTERN.sha.test(query.get('commit')) ? query.get('commit') : null;
    const dies = await load('/v1/dies').catch(() => ({ families: [] }));
    filters.replaceChildren(filterForm(chosen, dies.families ?? []));
    const search = new URLSearchParams();
    for (const [key, value] of Object.entries(chosen)) if (value !== null) search.set(key, value);
    const path = '/v1/matrix' + (search.size ? '?' + search : '');
    const data = await load(path);
    const suites = data.suites ?? [];
    const table = el('table', { class: 'kf-matrix' });
    table.append(
      el(
        'caption',
        {},
        'Latest run per die and suite' +
          (chosen.commit ? ' for commit ' + short(chosen.commit) : ''),
      ),
    );
    const groups = [];
    for (const suite of suites) {
      const last = groups[groups.length - 1];
      if (last && last.os === suite.os) last.count++;
      else groups.push({ os: suite.os, count: 1 });
    }
    table.append(
      el(
        'thead',
        {},
        el(
          'tr',
          {},
          el('th', { scope: 'col', rowspan: 2 }, 'Die'),
          groups.map((g) =>
            el(
              'th',
              { scope: 'colgroup', colspan: g.count },
              g.os === 'windows' ? 'Windows guest' : 'Linux guest',
            ),
          ),
        ),
        el(
          'tr',
          {},
          suites.map((s) => el('th', { scope: 'col' }, s.label)),
        ),
      ),
    );
    for (const family of data.families ?? []) {
      const body = el('tbody', {});
      body.append(
        el(
          'tr',
          { class: 'kf-family' },
          el('th', { scope: 'rowgroup', colspan: suites.length + 1 }, family.name),
        ),
      );
      for (const die of family.dies ?? []) {
        const row = el('tr', {}, el('th', { scope: 'row' }, die.id));
        for (const suite of suites) row.append(matrixCell(die, suite, die.cells?.[suite.id]));
        body.append(row);
      }
      table.append(body);
    }
    content.replaceChildren(
      el(
        'div',
        { class: 'kf-scroll', tabindex: 0, role: 'region', 'aria-label': 'Coverage matrix' },
        table,
      ),
      legend(),
    );
    const ref = data.reference ?? {};
    done('Reference: ' + (ref.branch ?? 'next') + (ref.head ? ' at ' + short(ref.head) : '') + '.');
  }

  function matrixCell(die, suite, cell) {
    const latest = cell?.latest ?? null;
    const pass = cell?.last_pass ?? null;
    const link = el(
      'a',
      { href: page('cell', { die: die.id, suite: suite.id }) },
      verdict(latest?.verdict, latest?.state === 'running'),
    );
    const lines = [link];
    if (latest?.tier) lines.push(el('span', { class: 'kf-tier' }, latest.tier));
    if (pass) {
      const changes = Number(pass.changes_since);
      lines.push(
        el(
          'span',
          { class: 'kf-pass' },
          'passed at ',
          commitLink(pass.commit),
          Number.isFinite(changes)
            ? changes === 0
              ? ', no test-relevant change since'
              : ', ' + changes + ' test-relevant change' + (changes === 1 ? '' : 's') + ' ago'
            : '',
        ),
      );
    } else if (latest) lines.push(el('span', { class: 'kf-pass' }, 'no pass on the branch'));
    const key =
      latest?.state === 'running'
        ? 'running'
        : VERDICTS.has(latest?.verdict)
          ? latest.verdict.toLowerCase()
          : 'none';
    return el('td', { class: 'kf-cell kf-c-' + key }, lines);
  }

  function legend() {
    return el(
      'p',
      { class: 'kf-legend' },
      'Verdicts: ',
      ['PASS', 'FAIL', 'RETRY', 'UNAVAILABLE'].map((v) => [verdict(v, false), ' ']),
      verdict(null, true),
      ' ',
      verdict(null, false),
      '. Tiers: largest (release), middle (master PR), small (driver matrix), adhoc.',
    );
  }

  function filterForm(chosen, families) {
    const select = (name, label, options) =>
      el(
        'label',
        {},
        label + ' ',
        el(
          'select',
          { name },
          el('option', { value: '' }, 'any'),
          options.map(([value, text]) =>
            el('option', { value, selected: chosen[name] === value }, text),
          ),
        ),
      );
    const form = el(
      'form',
      { class: 'kf-filters', method: 'get', action: page('', {}) },
      select(
        'family',
        'Family',
        families.map((f) => [f.id, f.name]),
      ),
      select('cpu', 'CPU', [
        ['intel', 'Intel'],
        ['amd', 'AMD'],
      ]),
      select('nested', 'Host', [
        ['1', 'nested VM'],
        ['0', 'bare metal'],
      ]),
      el(
        'label',
        {},
        'Host driver ',
        el('input', {
          name: 'driver',
          value: chosen.driver ?? '',
          size: 10,
          inputmode: 'decimal',
          pattern: '[0-9]{3}\\.[0-9]{1,3}(\\.[0-9]{1,3})?',
          placeholder: '580.159.04',
        }),
      ),
      el(
        'label',
        {},
        'Commit ',
        el('input', {
          name: 'commit',
          value: chosen.commit ?? '',
          size: 12,
          pattern: '[0-9a-f]{40}',
          placeholder: '40 hex digits',
        }),
      ),
      el('button', { type: 'submit' }, 'Apply'),
    );
    form.addEventListener('submit', (event) => {
      event.preventDefault();
      const values = {};
      for (const [key, value] of new FormData(form)) if (value) values[key] = value;
      location.href = page('', values);
    });
    return form;
  }

  // --- cell -----------------------------------------------------------------------------------
  async function cell() {
    const die = param('die');
    const suite = param('suite');
    if (!die || !suite) throw new Error('This page needs a die and a suite.');
    const before = param('before');
    const data = await load('/v1/cells/' + die + '/' + suite + (before ? '?before=' + before : ''));
    main.querySelector('h1').textContent = die + ': ' + (data.suite?.label ?? suite);
    const table = el(
      'table',
      { class: 'kf-runs' },
      el(
        'caption',
        {},
        'Runs of ' +
          (data.suite?.label ?? suite) +
          ' on ' +
          die +
          ' (' +
          (data.die?.family ?? '') +
          ')',
      ),
      el(
        'thead',
        {},
        el(
          'tr',
          {},
          ['Run', 'Verdict', 'Commit', 'Tier', 'Platform', 'Finished', 'Perf ratio'].map((h) =>
            el('th', { scope: 'col' }, h),
          ),
        ),
      ),
      el(
        'tbody',
        {},
        (data.runs ?? []).map((run) =>
          el(
            'tr',
            {},
            el('td', {}, el('a', { href: page('run', { id: run.id }) }, run.id)),
            el('td', {}, verdict(run.verdict, run.state === 'running')),
            el(
              'td',
              {},
              commitLink(run.commit),
              ' ',
              el(
                'a',
                { href: page('commit', { sha: run.commit }), class: 'kf-small' },
                'requirements',
              ),
            ),
            el('td', {}, run.tier),
            el('td', {}, platformText(run.platform)),
            el('td', {}, when(run.finished_at) || 'in flight'),
            el('td', {}, scoreText(run)),
          ),
        ),
      ),
    );
    const parts = [el('p', {}, el('a', { href: page('', {}) }, 'Back to the matrix'))];
    parts.push(
      (data.runs ?? []).length
        ? el(
            'div',
            { class: 'kf-scroll', tabindex: 0, role: 'region', 'aria-label': 'Runs' },
            table,
          )
        : el('p', {}, 'No runs yet.'),
    );
    if (data.next_before)
      parts.push(
        el(
          'p',
          {},
          el('a', { href: page('cell', { die, suite, before: data.next_before }) }, 'Older runs'),
        ),
      );
    content.replaceChildren(...parts);
    done(
      (data.runs ?? []).length +
        ' runs shown. Perf ratios compare with a same-box reference and never gate.',
    );
  }

  // --- run ------------------------------------------------------------------------------------
  async function run() {
    const id = param('id');
    if (!id) throw new Error('This page needs a run id.');
    const data = (await load('/v1/runs/' + id)).run;
    main.querySelector('h1').textContent = 'Run ' + data.id;
    const facts = [
      ['Verdict', verdict(data.verdict, data.state === 'running')],
      [
        'Die',
        el(
          'a',
          { href: page('cell', { die: data.die, suite: data.suite }) },
          data.die + ' (' + data.family + ')',
        ),
      ],
      ['Suite', data.suite],
      [
        'Commit',
        [
          commitLink(data.commit),
          ' ',
          data.commit_info?.message ?? '',
          ' ',
          el('a', { href: page('commit', { sha: data.commit }) }, 'requirements'),
        ],
      ],
      ['Started', when(data.started_at)],
      ['Finished', when(data.finished_at) || 'in flight'],
      ['Platform', platformText(data.platform)],
      ['CPU', data.platform?.cpu_model],
      ['Host kernel', data.platform?.kernel],
      ['QEMU build', data.platform?.qemu_build],
      ['IOMMU mode', data.platform?.iommu_mode],
      ['Perf ratio', scoreText(data) || 'none'],
      ['Source', data.provenance?.source],
      ['Tier', data.provenance?.tier],
      ['Requirement', data.provenance?.requirement],
      ['Boundary hash', el('code', {}, data.provenance?.boundary_hash ?? '')],
    ];
    const list = el(
      'dl',
      { class: 'kf-facts' },
      facts.map(([k, v]) => [el('dt', {}, k), el('dd', {}, v)]),
    );
    const host = el(
      'dl',
      { class: 'kf-facts' },
      Object.entries(data.host_facts ?? {}).map(([k, v]) => [
        el('dt', {}, k),
        el('dd', {}, String(v)),
      ]),
    );
    const stages = el(
      'table',
      { class: 'kf-runs' },
      el('caption', {}, 'Stages'),
      el(
        'thead',
        {},
        el(
          'tr',
          {},
          ['Stage', 'Verdict', 'Started', 'Finished'].map((h) => el('th', { scope: 'col' }, h)),
        ),
      ),
      el(
        'tbody',
        {},
        (data.stages ?? []).map((s) =>
          el(
            'tr',
            {},
            el('td', {}, s.name),
            el(
              'td',
              {},
              s.verdict === 'SKIPPED'
                ? el('span', { class: 'kf-v kf-v-none' }, 'skipped')
                : verdict(s.verdict, s.verdict === null),
            ),
            el('td', {}, when(s.started_at)),
            el('td', {}, when(s.finished_at)),
          ),
        ),
      ),
    );
    const logList = el(
      'ul',
      {},
      (data.logs ?? []).map((log) => {
        if (mode === 'fixture') return el('li', {}, log.name + ' (no log files in the fixture)');
        const href = logs
          ? logs + '/runs/' + encodeURIComponent(data.id) + '/' + encodeURIComponent(log.name)
          : api + log.path;
        return el('li', {}, el('a', { href, type: 'text/plain' }, log.name));
      }),
    );
    content.replaceChildren(
      list,
      el('h2', {}, 'Host facts'),
      host,
      el('h2', {}, 'Stages'),
      el(
        'div',
        { class: 'kf-scroll', tabindex: 0, role: 'region', 'aria-label': 'Stages' },
        stages,
      ),
      el('h2', {}, 'Logs'),
      (data.logs ?? []).length ? logList : el('p', {}, 'No logs.'),
    );
    done(data.state === 'running' ? 'This run is still in flight.' : 'This run is final.');
  }

  // --- commit ---------------------------------------------------------------------------------
  async function commit() {
    const sha = param('sha');
    if (!sha) throw new Error('This page needs a 40-digit commit id.');
    const data = await load('/v1/commits/' + sha + '/requirements');
    main.querySelector('h1').textContent = 'Commit ' + short(sha);
    const items = (data.requirements ?? []).map((q) =>
      el(
        'li',
        { class: 'kf-req' },
        el('span', { class: 'kf-v kf-v-' + q.state.toLowerCase().replace('_', '-') }, q.state),
        ' ',
        el('strong', {}, q.key),
        q.run ? [' answered by ', el('a', { href: page('run', { id: q.run.id }) }, q.run.id)] : '',
      ),
    );
    content.replaceChildren(
      el(
        'p',
        {},
        commitLink(sha),
        ' ',
        data.commit?.message ?? '',
        ' (',
        data.commit?.author ?? '',
        ', ',
        when(data.commit?.committed_at),
        ')',
      ),
      el(
        'p',
        {},
        'On: ' + ((data.branches ?? []).map((b) => b.branch).join(', ') || 'no tracked branch'),
      ),
      items.length
        ? el('ul', { class: 'kf-checklist' }, items)
        : el('p', {}, 'No requirement results for this commit.'),
    );
    done(items.length + ' requirements.');
  }

  const views = { matrix, cell, run, commit };
  const view = views[main.dataset.page];
  if (view) view().catch(fail);
})();
