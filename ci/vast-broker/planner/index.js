// Public pure ESM surface. No provider calls, clock, randomness, storage or Workers APIs.
export { PlanError, contractSchema, checkSchema } from './schema.js';
export { resolveDependencies } from './model.js';
export { validateInput, validatePlan, validateAssignment } from './validate.js';
import { planPrepared as buildPlan, withoutSeedsPrepared as removeEdges } from './plan.js';
import { canonical } from './schema.js';
import { validatePrepared } from './validate.js';
import { prepare } from './model.js';
export function plan(input) {
  const ctx = prepare(input),
    result = buildPlan(ctx);
  validatePrepared(ctx, result, { forExecution: false });
  return JSON.parse(canonical(result));
}
export function withoutSeeds(input, p) {
  const ctx = prepare(input);
  validatePrepared(ctx, p, { forExecution: false });
  const cold = removeEdges(ctx, p);
  validatePrepared(ctx, cold, { forExecution: false });
  return JSON.parse(canonical(cold));
}
