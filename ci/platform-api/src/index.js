// kayfabe public platform API (api.kayfabe.dev): read-only, GET only, no secrets, no outbound
// requests. Design: docs/design/V3_PLATFORM_TESTING.md. Bindings: DB (D1, kayfabe-platform);
// optional var LOGS_BASE (https origin of the public log bucket on kayfabe.dev).
import { handle } from './api.js';

export default {
  async fetch(request, env) {
    return handle(request, env);
  },
};
