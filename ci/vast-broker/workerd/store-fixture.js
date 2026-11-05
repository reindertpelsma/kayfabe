// Test-only entrypoint for the result store in workerd with a local D1 (Miniflare).
import { recordRun, requirementsMet } from '../src/results.js';

export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    const body = await request.json();
    try {
      if (path === '/record') return Response.json(await recordRun(env, body));
      if (path === '/check') return Response.json(await requirementsMet(env, body));
    } catch (error) {
      return Response.json({ error: String(error.message) }, { status: 400 });
    }
    return new Response('', { status: 404 });
  },
};
