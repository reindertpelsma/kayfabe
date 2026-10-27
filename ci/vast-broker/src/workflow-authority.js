// Pure workflow authority: only this repository's pinned master workflows, each with the
// events its YAML may start. Who triggered the run is checked against one GitHub run read.
export function allowedWorkflowRef(c, cfg, profile = 'any') {
  if (
    !c ||
    c.repository !== cfg.repo ||
    typeof c.workflow_ref !== 'string' ||
    c.job_workflow_ref !== c.workflow_ref ||
    !['any', 'check', 'sweep'].includes(profile)
  )
    return false;
  const check = cfg.repo + '/.github/workflows/gpu-vast.yml@refs/heads/master';
  const sweep = cfg.repo + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master';
  if (c.workflow_ref === check)
    return profile !== 'sweep' && ['pull_request_target', 'push'].includes(c.event_name);
  if (c.workflow_ref === sweep) return profile !== 'check' && c.event_name === 'workflow_dispatch';
  return false;
}
