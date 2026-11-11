import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import worker, { normalisePrefix, parsePage } from "../src/index.js";

function fakeBucket(keys) {
  const sorted = [...keys].sort();
  const b = {
    calls: [],
    async list({ prefix = "", delimiter, limit = 1000, cursor }) {
      b.calls.push({ prefix, delimiter, limit, cursor });
      const entries = [];
      const seen = new Set();
      for (const k of sorted) {
        if (!k.startsWith(prefix)) continue;
        const rest = k.slice(prefix.length);
        const i = delimiter ? rest.indexOf(delimiter) : -1;
        if (i >= 0) {
          const d = prefix + rest.slice(0, i + 1);
          if (seen.has(d)) continue;
          seen.add(d);
          entries.push({ folder: d });
        } else entries.push({ obj: { key: k, size: k.length * 10, uploaded: new Date(Date.UTC(2026, 10, 1)) } });
      }
      const start = cursor ? Number(cursor.replace("c", "")) : 0;
      const slice = entries.slice(start, start + limit);
      const end = start + limit;
      return {
        objects: slice.filter((e) => e.obj).map((e) => e.obj),
        delimitedPrefixes: slice.filter((e) => e.folder).map((e) => e.folder),
        truncated: end < entries.length,
        cursor: end < entries.length ? `c${end}` : undefined,
      };
    },
  };
  return b;
}

let store, fetched;
beforeEach(() => {
  store = new Map();
  fetched = [];
  globalThis.caches = {
    default: {
      async match(req) {
        const r = store.get(req.url);
        return r ? r.clone() : undefined;
      },
      async put(req, res) {
        store.set(req.url, res);
      },
    },
  };
  globalThis.fetch = async (req) => {
    fetched.push(req);
    return new Response("origin");
  };
});

function env(logs, bundles = fakeBucket([])) {
  return {
    BUCKET_LOGS: logs,
    BUCKET_BUNDLES: bundles,
    HOSTS: '{"logs.kayfabe.dev":"BUCKET_LOGS","bundles.kayfabe.dev":"BUCKET_BUNDLES"}',
  };
}
const get = (e, url, method = "GET") => worker.fetch(new Request(url, { method }), e);

test("file path passes through unchanged", async () => {
  const e = env(fakeBucket(["a/b.txt"]));
  const r = await get(e, "https://logs.kayfabe.dev/a/b.txt");
  assert.equal(await r.text(), "origin");
  assert.equal(fetched.length, 1);
  assert.equal(e.BUCKET_LOGS.calls.length, 0);
  // folder without trailing slash is not special
  await get(e, "https://logs.kayfabe.dev/a");
  assert.equal(fetched.length, 2);
});

test("root and nested listing, folders first, hidden omitted", async () => {
  const e = env(fakeBucket(["z.txt", "run-1/a.log", "run-1/sub/x", "run-1/.hid", "run-1/_priv", ".top", "run-2/b.log"]));
  let r = await get(e, "https://logs.kayfabe.dev/");
  assert.equal(r.status, 200);
  assert.match(r.headers.get("content-security-policy"), /default-src 'none'/);
  assert.equal(r.headers.get("x-content-type-options"), "nosniff");
  let h = await r.text();
  assert.ok(h.indexOf("run-1/") < h.indexOf("z.txt"));
  assert.ok(!h.includes(".top"));
  r = await get(e, "https://logs.kayfabe.dev/run-1/");
  h = await r.text();
  assert.match(h, /href="\/run-1\/a\.log"/);
  assert.match(h, /href="\/run-1\/sub\/"/);
  assert.ok(!h.includes(".hid") && !h.includes("_priv"));
  assert.deepEqual(e.BUCKET_LOGS.calls[1], { prefix: "run-1/", delimiter: "/", limit: 100, cursor: undefined });
});

test("bundles host uses its own bucket", async () => {
  const e = env(fakeBucket(["l.txt"]), fakeBucket(["some/dir/f.bin"]));
  const h = await (await get(e, "https://bundles.kayfabe.dev/some/dir/")).text();
  assert.match(h, /f\.bin/);
  assert.ok(!h.includes("l.txt"));
});

test("paging: cursor chain and bounded list calls on a crawl", async () => {
  const keys = Array.from({ length: 250 }, (_, i) => `f${String(i).padStart(4, "0")}`);
  const b = fakeBucket(keys);
  const e = env(b);
  let h = await (await get(e, "https://logs.kayfabe.dev/")).text();
  assert.match(h, /page 1/);
  assert.equal(b.calls.length, 1);
  h = await (await get(e, "https://logs.kayfabe.dev/?page=3")).text();
  assert.match(h, /f0200/);
  assert.match(h, /page 3 of 3/);
  assert.equal(b.calls.length, 3); // walked 2 and 3
  // crawl: pages 1 and 3 are cached; page 2 costs ONE call from its cached cursor; repeats cost none
  for (const p of [1, 2, 3, 1, 2, 3]) await get(e, `https://logs.kayfabe.dev/?page=${p}`);
  assert.equal(b.calls.length, 4);
  assert.equal(b.calls[3].cursor, "c100");
  // page 1 was rendered before the total was known (stale for up to the TTL); once expired it links the last page
  for (const k of [...store.keys()]) if (k.includes("/page/")) store.delete(k);
  h = await (await get(e, "https://logs.kayfabe.dev/?page=1")).text();
  assert.equal(b.calls.length, 5);
  assert.match(h, /last page \(newest\)/);
  assert.match(h, /href="\/\?page=3"/);
  // cursor chain reuse: page cache is empty again, page 3 needs one call from its cached cursor
  for (const k of [...store.keys()]) if (k.includes("/page/")) store.delete(k);
  await get(e, "https://logs.kayfabe.dev/?page=3");
  assert.equal(b.calls.length, 6);
  assert.equal(b.calls[5].cursor, "c200");
  // beyond the end
  assert.equal((await get(e, "https://logs.kayfabe.dev/?page=4")).status, 404);
});

test("HEAD on listing has no body", async () => {
  const e = env(fakeBucket(["a"]));
  const r = await get(e, "https://logs.kayfabe.dev/", "HEAD");
  assert.equal(r.status, 200);
  assert.equal(await r.text(), "");
  const r2 = await get(e, "https://logs.kayfabe.dev/", "HEAD");
  assert.equal(await r2.text(), "");
});

test("hostile paths and page values", async () => {
  const e = env(fakeBucket(["a/b"]));
  const bad = [
    "/a//b/", "/a%00/", "/a%0a/", "/%5c/", "/" + "x/".repeat(300), "/%zz/"

  ];
  for (const p of bad) assert.equal((await get(e, "https://logs.kayfabe.dev" + p)).status, 404, p);
  for (const q of ["page=0", "page=-1", "page=abc", "page=1001", "page=1&page=2", "page=1.5", "page="]) {
    assert.equal((await get(e, `https://logs.kayfabe.dev/?${q}`)).status, 404, q);
  }
  assert.equal(e.BUCKET_LOGS.calls.length, 0);
  assert.equal(normalisePrefix("/a/b/"), "a/b/");
  for (const p of ["/../", "/a/../", "/a/./", "/a/%2e%2e/"]) assert.equal(normalisePrefix(p), null, p);
  assert.equal(parsePage(""), 1);
});

test("HTML escaping of keys", async () => {
  const e = env(fakeBucket(['<script>"x"&.txt', "a'b/c"]));
  const h = await (await get(e, "https://logs.kayfabe.dev/")).text();
  assert.ok(!h.includes("<script>"));
  assert.match(h, /&lt;script&gt;&quot;x&quot;&amp;\.txt/);
  assert.match(h, /a&#39;b\//);
  assert.match(h, /href="\/%3Cscript%3E/);
});

test("unknown host 404 and no bucket access", async () => {
  const e = env(fakeBucket(["a"]));
  assert.equal((await get(e, "https://evil.example/")).status, 404);
  assert.equal((await get(e, "https://evil.example/file")).status, 404);
  assert.equal(fetched.length, 0);
  assert.equal(e.BUCKET_LOGS.calls.length, 0);
});

test("only page is honoured; other params share the cache", async () => {
  const b = fakeBucket(["a"]);
  const e = env(b);
  await get(e, "https://logs.kayfabe.dev/?cursor=zzz&limit=1&prefix=x&page=1");
  await get(e, "https://logs.kayfabe.dev/?foo=bar");
  await get(e, "https://logs.kayfabe.dev/");
  assert.equal(b.calls.length, 1);
  assert.equal(b.calls[0].cursor, undefined);
  assert.equal(b.calls[0].limit, 100);
});

test("non GET/HEAD is 405", async () => {
  const e = env(fakeBucket([]));
  assert.equal((await get(e, "https://logs.kayfabe.dev/", "POST")).status, 405);
});
