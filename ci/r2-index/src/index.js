// Folder-style index pages for public R2 buckets. Design: ci/r2-index/README.md.
// Imports nothing; talks to no other Worker.

export const PAGE_SIZE = 100;
export const MAX_PAGE = 1000;
export const TTL = 300;
// Cold listings (cache misses that call R2) allowed per host per minute per data center. A crawler asking for
// many different made-up folders gets 429 instead of an unbounded number of Class A operations.
export const COLD_LIMIT = 30;
export const MAX_PREFIX = 512;
// Objects whose name (last path segment) starts with one of these are not listed.
export const HIDDEN_PREFIXES = [".", "_"];

const HEADERS = {
  "content-type": "text/html; charset=utf-8",
  "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'",
  "x-content-type-options": "nosniff",
  "referrer-policy": "no-referrer",
  "x-robots-tag": "noindex, nofollow, noarchive",
};

const ROBOTS = "User-agent: *\nDisallow: /\n";

export function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

// Returns the prefix ("" or "a/b/") for a pathname ending in '/', or null if rejected.
export function normalisePrefix(pathname) {
  let p;
  try {
    p = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (!p.startsWith("/") || !p.endsWith("/")) return null;
  p = p.slice(1);
  if (p.length > MAX_PREFIX) return null;
  if (/[\u0000-\u001f\u007f\\]/.test(p)) return null;
  if (p === "") return "";
  if (p.startsWith("/") || p.includes("//")) return null;
  if (p.slice(0, -1).split("/").some((s) => s === ".." || s === ".")) return null;
  return p;
}

// Only `page` is honoured. Returns the page number, or null when invalid.
export function parsePage(search) {
  const vals = new URLSearchParams(search).getAll("page");
  if (vals.length === 0) return 1;
  if (vals.length > 1 || !/^[0-9]{1,4}$/.test(vals[0])) return null;
  const n = Number(vals[0]);
  return n >= 1 && n <= MAX_PAGE ? n : null;
}

function hidden(name) {
  return HIDDEN_PREFIXES.some((h) => name.startsWith(h));
}

function pickBucket(env, host) {
  let map;
  try {
    map = JSON.parse(env.HOSTS || "{}");
  } catch {
    return null;
  }
  if (!Object.prototype.hasOwnProperty.call(map, host)) return null;
  return env[map[host]] || null;
}

// Cache keys are synthetic URLs; the query string never comes from the client.
function cacheKey(kind, host, prefix, page) {
  const u = new URL("https://r2-index.invalid/");
  u.pathname = `/${kind}/${encodeURIComponent(host)}/${encodeURIComponent(prefix)}`;
  if (page !== undefined) u.searchParams.set("page", String(page));
  return new Request(u.toString());
}

async function cacheGetJson(cache, key) {
  const r = await cache.match(key);
  return r ? r.json() : null;
}

function cachePutJson(cache, key, value) {
  return cache.put(
    key,
    new Response(JSON.stringify(value), { headers: { "cache-control": `public, max-age=${TTL}`, "content-type": "application/json" } }),
  );
}

// One R2 list call yields up to PAGE_SIZE entries (folders + files). Hidden entries are removed after the
// call, so a page may hold slightly fewer. `cursor` undefined = first page.
async function listPage(bucket, prefix, cursor) {
  const res = await bucket.list({ prefix, delimiter: "/", limit: PAGE_SIZE, cursor });
  const folders = (res.delimitedPrefixes || []).filter((f) => !hidden(f.slice(prefix.length)));
  const files = (res.objects || [])
    .filter((o) => o.key !== prefix && !hidden(o.key.slice(prefix.length)))
    .map((o) => ({ key: o.key, size: o.size, uploaded: o.uploaded ? new Date(o.uploaded).toISOString() : "" }));
  return { folders, files, next: res.truncated ? res.cursor : null };
}

function fmtSize(n) {
  if (typeof n !== "number") return "";
  if (n < 1024) return `${n} B`;
  const u = ["KiB", "MiB", "GiB", "TiB"];
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < u.length - 1);
  return `${v.toFixed(1)} ${u[i]}`;
}

function href(key) {
  return "/" + key.split("/").map(encodeURIComponent).join("/");
}

function render(host, prefix, page, data, totalPages) {
  const crumbs = [`<a href="/">${esc(host)}</a>`];
  let acc = "";
  for (const p of prefix.split("/").filter(Boolean)) {
    acc += p + "/";
    crumbs.push(`<a href="${esc(href(acc))}">${esc(p)}</a>`);
  }
  const base = href(prefix);
  const link = (n, label) => `<a href="${esc(n === 1 ? base : `${base}?page=${n}`)}">${esc(label)}</a>`;
  const nav = [];
  if (page > 1) nav.push(link(page - 1, "previous"));
  nav.push(`page ${page}${totalPages ? ` of ${totalPages}` : ""}`);
  if (data.next) nav.push(link(page + 1, "next"));
  if (totalPages && page < totalPages) nav.push(`<strong>${link(totalPages, "last page (newest)")}</strong>`);
  else if (data.next) nav.push("last page: not yet known");
  const rows = [];
  for (const f of data.folders) {
    rows.push(`<tr><td><a href="${esc(href(f))}">${esc(f.slice(prefix.length))}</a></td><td>folder</td><td></td></tr>`);
  }
  for (const f of data.files) {
    rows.push(
      `<tr><td><a href="${esc(href(f.key))}">${esc(f.key.slice(prefix.length))}</a></td><td>${esc(fmtSize(f.size))}</td><td>${esc(f.uploaded)}</td></tr>`,
    );
  }
  if (!rows.length) rows.push('<tr><td colspan="3">empty</td></tr>');
  const navHtml = `<nav>${nav.join(" | ")}</nav>`;
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>Index of /${esc(prefix)}</title><style>body{font:14px/1.5 system-ui,sans-serif;margin:1rem auto;max-width:60rem;padding:0 1rem}td,th{padding:.15rem .8rem .15rem 0;text-align:left}nav{margin:.6rem 0}</style></head><body><h1>Index of /${esc(prefix)}</h1><nav>${crumbs.join(" / ")}</nav>${navHtml}<table><thead><tr><th>Name</th><th>Size</th><th>Uploaded</th></tr></thead><tbody>${rows.join("")}</tbody></table>${navHtml}</body></html>`;
}

function notFound() {
  return new Response("not found\n", { status: 404, headers: { ...HEADERS, "content-type": "text/plain; charset=utf-8" } });
}

async function coldBudgetOk(cache, host) {
  const key = cacheKey("budget", host, String(Math.floor(Date.now() / 60000)));
  const cur = (await cacheGetJson(cache, key)) || { n: 0 };
  if (cur.n >= COLD_LIMIT) return false;
  cur.n++;
  await cachePutJson(cache, key, cur);
  return true;
}

function busy() {
  return new Response("busy, retry later\n", {
    status: 429,
    headers: { ...HEADERS, "content-type": "text/plain; charset=utf-8", "retry-after": "60" },
  });
}

async function listing(request, bucket, host, prefix, page) {
  const cache = caches.default;
  const head = request.method === "HEAD";
  const pageKey = cacheKey("page", host, prefix, page);
  const hit = await cache.match(pageKey);
  if (hit) return new Response(head ? null : hit.body, { status: 200, headers: HEADERS });

  const chainKey = cacheKey("chain", host, prefix);
  const countKey = cacheKey("count", host, prefix);
  // cursors[k] is the cursor that fetches page k+2.
  const chain = (await cacheGetJson(cache, chainKey)) || { cursors: [] };
  let total = await cacheGetJson(cache, countKey);
  // A page past the known end costs nothing: no R2 list call (a crawler asking for page 2..1000 of a small
  // folder must not spend Class A operations on every request).
  if (total != null && page > total) return null;
  if (!(await coldBudgetOk(cache, host))) return busy();
  // Walk from the furthest cached cursor at or before the wanted page.
  let reached = Math.min(page, chain.cursors.length + 1);
  let cursor = reached > 1 ? chain.cursors[reached - 2] : undefined;
  let data;
  for (;;) {
    data = await listPage(bucket, prefix, cursor);
    if (data.next) {
      chain.cursors[reached - 1] = data.next;
    } else {
      chain.cursors.length = Math.min(chain.cursors.length, reached - 1);
      total = reached;
    }
    if (reached === page) break;
    if (!data.next) {
      // Beyond the end: remember the end so repeating the request is free.
      await Promise.all([cachePutJson(cache, chainKey, chain), cachePutJson(cache, countKey, total)]);
      return null;
    }
    cursor = data.next;
    reached++;
  }
  await Promise.all([
    cachePutJson(cache, chainKey, chain),
    total != null ? cachePutJson(cache, countKey, total) : Promise.resolve(),
  ]);
  // A folder that does not exist (or holds only hidden entries) is a 404, not an empty index.
  if (prefix !== "" && page === 1 && data.folders.length === 0 && data.files.length === 0) return null;
  const html = render(host, prefix, page, data, total);
  await cache.put(pageKey, new Response(html, { headers: { ...HEADERS, "cache-control": `public, max-age=${TTL}` } }));
  return new Response(head ? null : html, { status: 200, headers: HEADERS });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const host = url.hostname.toLowerCase();
    const bucket = pickBucket(env, host);
    if (!bucket) return notFound();
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("method not allowed\n", { status: 405, headers: { ...HEADERS, allow: "GET, HEAD" } });
    }
    if (url.pathname === "/robots.txt") {
      return new Response(request.method === "HEAD" ? null : ROBOTS, {
        status: 200,
        headers: { ...HEADERS, "content-type": "text/plain; charset=utf-8", "cache-control": "public, max-age=3600" },
      });
    }
    if (!url.pathname.endsWith("/")) {
      const origin = await fetch(request);
      const out = new Response(origin.body, origin);
      out.headers.set("x-robots-tag", HEADERS["x-robots-tag"]);
      return out;
    }
    const prefix = normalisePrefix(url.pathname);
    const page = parsePage(url.search);
    if (prefix === null || page === null) return notFound();
    return (await listing(request, bucket, host, prefix, page)) || notFound();
  },
};
