// Pure workflow authority. The API row is trusted only after an authenticated GitHub read.
// Master pinning alone does not approve a rental; admission still checks actor/commit policy.
export function allowedWorkflowRef(c, cfg, run, profile = 'any') {
  if (
    !c ||
    c.repository !== cfg.repo ||
    typeof c.workflow_ref !== 'string' ||
    c.workflow_ref.length > 2048 ||
    c.job_workflow_ref !== c.workflow_ref ||
    !['any', 'check', 'sweep'].includes(profile)
  )
    return false;
  const check = cfg.repo + '/.github/workflows/gpu-vast.yml@refs/heads/master';
  const sweep = cfg.repo + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master';
  if (c.workflow_ref === check) return profile !== 'sweep';
  if (c.workflow_ref === sweep) return profile !== 'check';
  if (profile === 'sweep') return false;
  const delimiter = c.workflow_ref.indexOf('@');
  const path = c.workflow_ref.slice(0, delimiter);
  const ref = c.workflow_ref.slice(delimiter + 1);
  if (path !== cfg.repo + '/.github/workflows/gpu-vast.yml') return false;
  if (!ref.startsWith('refs/heads/') || ref.length <= 11) return false;
  if (ref !== c.ref || !run || typeof run.head_branch !== 'string') return false;
  const ownerActor = c.actor_id === cfg.owner && String(run.actor?.id) === cfg.owner;
  if (!ownerActor) return false;
  if (String(run.triggering_actor?.id) !== cfg.owner) return false;
  if (run.head_repository?.full_name !== cfg.repo) return false;
  if (run.repository?.full_name !== cfg.repo) return false;
  if (String(run.id) !== c.run_id || String(run.run_attempt) !== c.run_attempt) return false;
  if (run.event !== c.event_name || run.head_sha !== c.sha) return false;
  return ref.split('/').slice(2).join('/') === run.head_branch;
}
