// Response helpers. GET responses carry CORS `*`; nothing else does.

export const CACHE = Object.freeze({
  immutable: 'public, max-age=31536000, immutable',
  short: 'public, max-age=30',
  // The expensive read endpoints (/v1/ci/subruns, /v1/ci/stats): a shared cache may hold them a minute.
  edge: 'public, max-age=30, s-maxage=60',
  static: 'public, max-age=300',
  none: 'no-store',
});

const COMMON = {
  'X-Content-Type-Options': 'nosniff',
  'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
  'Referrer-Policy': 'no-referrer',
};

export function json(body, { status = 200, cache = CACHE.short } = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      ...COMMON,
      'Content-Type': 'application/json; charset=utf-8',
      'Cache-Control': cache,
      'Access-Control-Allow-Origin': '*',
    },
  });
}

export function redirect(location) {
  return new Response(null, {
    status: 302,
    headers: {
      ...COMMON,
      Location: location,
      'Cache-Control': CACHE.static,
      'Access-Control-Allow-Origin': '*',
    },
  });
}

export function errorResponse(status, code, detail, extra = {}) {
  return json({ error: code, detail, ...extra }, { status, cache: CACHE.none });
}

// Non-GET: no CORS header, no body beyond a fixed error.
export function methodNotAllowed() {
  return new Response(JSON.stringify({ error: 'method_not_allowed', detail: 'GET only' }), {
    status: 405,
    headers: {
      ...COMMON,
      'Content-Type': 'application/json; charset=utf-8',
      'Cache-Control': CACHE.none,
      Allow: 'GET',
    },
  });
}
