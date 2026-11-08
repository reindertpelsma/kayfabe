import { plan } from '../index.js';
import { seeded } from './seed-fixture.js';
import { createSession, startAssignment } from '../artifacts.js';
export function protocolFixture(n = 1, consumerProvider = 'local', content = null) {
  const input = seeded(n);
  if (content)
    Object.assign(
      input.catalog.artifacts.find((a) => a.id === 'open_image').trusted_content,
      content,
    );
  for (const b of input.boxes)
    if (input.requirements.items.some((q) => q.tier === 'medium' && q.die === b.die))
      b.provider = consumerProvider;
  const output = plan(input);
  const source = output.assignments.find((a) => a.seed_exports.length);
  const consumer = output.assignments.find((a) => a.seed_hints.length);
  const hint = consumer.seed_hints.find((h) => h.artifact === 'open_image');
  let state = createSession(output, 'release-test');
  state = startAssignment(state, source.id, 'source-instance', source.start_minute);
  const now = hint.available_minute;
  const ctx = {
    input,
    plan: output,
    session_id: 'release-test',
    admission_open: true,
    worker_url: 'https://worker.example',
    auth: {
      verified: true,
      session_id: 'release-test',
      assignment_id: source.id,
      instance_id: 'source-instance',
      expires_minute: 300,
    },
    facts: {
      assignment_id: source.id,
      instance_id: 'source-instance',
      provider: source.provider,
      public_ip: '8.8.8.8',
      mapped_ports: [{ container_port: 8080, public_port: 45321, protocol: 'tcp' }],
      requested_ports: [8080],
      observed_minute: now,
    },
  };
  const body = {
    version: 1,
    artifact: 'open_image',
    url: 'http://8.8.8.8:45321/abcdefgh/open-image.tar.zst',
    sha256: hint.trusted_content.sha256,
    size: hint.trusted_content.size,
  };
  const consumerContext = () => ({
    ...ctx,
    auth: { ...ctx.auth, assignment_id: consumer.id, instance_id: 'consumer-instance' },
  });
  return { input, plan: output, source, consumer, hint, state, ctx, body, now, consumerContext };
}
