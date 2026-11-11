# r2-index: folder listings for the public R2 buckets

STATUS: DESIGN, NOT DEPLOYED, 2026-11-11. Built and unit-tested GPU-free only (`node --test`, fake R2 bucket).

A small standalone Cloudflare Worker. It gives `https://logs.kayfabe.dev/`, `https://logs.kayfabe.dev/vast-123-1-0/`
and `https://bundles.kayfabe.dev/some/dir/` a small HTML index page under the same paths as the files.
It imports nothing from `ci/vast-broker` or `ci/platform-api`, has no runtime dependencies, has no secrets and never
talks to the main Worker.

## Behaviour

- A path that does not end in `/` (a file): `fetch(request)` unchanged, so the origin's headers and CDN caching stay
  as they are today. A folder name without the trailing slash is not special (the origin answers 404).
- `/` or a path ending in `/`: `bucket.list({prefix, delimiter: "/", limit: 100})`, rendered as one page with
  breadcrumbs, folders first, then files (name, size, upload date), `previous` / `next` links and a visible
  `last page (newest)` link once the page count is known (R2 lists ascending, run ids grow).
- Only `?page=N` (1..1000) is read. Any other query parameter is ignored and is not part of any cache key. An invalid
  `page` (0, negative, non-numeric, repeated, above 1000) is 404, as is a page past the end. There is no cursor from the client.
- The Worker maps page to R2 cursor itself. Per (host, prefix) it caches, in `caches.default` for 300 s: the cursor
  chain, the page count and every rendered page. An uncached page is reached from the furthest cached cursor, so a
  crawler walking all pages costs about one list call per page per 300 s.
- Prefix is rejected (404) when it contains `..` or `.` segments, control characters, a backslash, `//`, or exceeds 512 chars.
- Every key is HTML-escaped; links are percent-encoded. Headers: `Content-Security-Policy: default-src 'none';
  style-src 'unsafe-inline'`, `X-Content-Type-Options: nosniff`. No JSON API. HEAD on a listing returns headers only.
  The Worker only reads the bucket bound for the request hostname; an unknown host is 404.
- Objects whose name starts with `.` or `_` are not listed (`HIDDEN_PREFIXES` in `src/index.js`). They are still
  reachable by direct URL as before. A page may therefore hold fewer than 100 entries.
- Methods other than GET and HEAD: 405.

## wrangler.toml notes

- Routes `logs.kayfabe.dev/*` and `bundles.kayfabe.dev/*` (zone `kayfabe.dev`); both hostnames stay R2 custom domains.
- R2 bindings `BUCKET_LOGS` (`kayfabe-logs-public`) and `BUCKET_BUNDLES` (`kayfabe-bundles-public`).
- `HOSTS` var: JSON map hostname to binding name. Adding a bucket is a binding plus a map entry.

## Cost

A listing is an R2 Class A operation, bounded by the caches above (the 300 s TTL, one call per page). File downloads pass
through to the origin, so they are not billed as the Worker serving file bytes; each still counts as one Worker request.

## Unverified

- UNVERIFIED: Worker route in front of an R2 custom domain with pass-through fetch(request) works as assumed.
  The coordinator will test it on a throwaway path before deploy.
- UNVERIFIED: `caches.default` behaviour on workers.dev-less routes and the exact TTL honoured; the tests use a fake cache.

## Tests

`cd ci/r2-index && node --test test/*.test.js` (Node 22.13 or newer).

## Crawlers

The Worker answers `/robots.txt` itself on both hosts (`User-agent: *`, `Disallow: /`) and adds
`X-Robots-Tag: noindex, nofollow, noarchive` to every listing and to every passed-through file. This is a
request to well-behaved crawlers, not access control: nothing non-public belongs in these buckets. Until
the Worker is deployed, the same `robots.txt` exists as an object in both buckets (uploaded 2026-11-11).
