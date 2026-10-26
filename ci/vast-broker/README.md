# Vast GPU CI broker

The broker connects GitHub Actions to rented NVIDIA GPU machines on Vast.ai so kayfabe can run GPU checks beyond the hardware available to hosted runners. It is a Cloudflare Worker backed by a Durable Object ledger that tracks rentals, deadlines and spending. GitHub asks for a check; the Worker handles the provider account and relays the box's result and bounded logs back to the runner.

## How a GPU check uses it

1. The [GPU workflow](../../.github/workflows/gpu-vast.yml) decides whether a master push or PR needs a check. It fetches the controller from the trusted master base, without checking out proposed code on the GitHub runner. Two independent path decisions must agree before it reports a check as not needed.
2. The controller presents GitHub OIDC identity. The Worker checks the pinned workflow, the run's owner triggering actor and the exact tested commit, then applies offer eligibility, concurrency and spending policy before creating a rental.
3. The box bootstrap checks out that exact commit and runs the [repository-side payload and reporter](../../scripts/ci/README.md). Its scoped run token lets it report progress, logs and a structured result, request its own destruction, and obtain permitted private asset download URLs.
4. The GitHub controller independently validates the result bindings and log hashes, scrubs displayed output and supplies the workflow's `gpu-vast` status. After fetching the final log it requests immediate destruction; result grace and Worker deadlines provide fallback cleanup.

## Trust and spending boundaries

The Vast box and its operator are untrusted. The Vast account key stays in the Worker, outside GitHub secrets and rented machines. A box receives only its run capability and, when configured, a Cloudflare Access service-token pair. GitHub communicates through the broker and accepts no executable or build artifact from the box. A GPU PASS is an unattested tripwire: a dishonest box or Worker can forge a verdict. It supplies no release provenance. See the [trust and release trace](../../docs/design/V3_CI_VAST_RUNNER_TRACE_20261022.md).

The ledger reserves liability before rental and applies daily and rolling budgets, a prepaid credit reserve and limits on concurrent work. The default check policy is $1 effective/hour, $6/day, $30 per rolling 30 days and an $8 credit reserve, with one active rental and a three-hour hard deadline. The separate owner-dispatched sweep profile has its own hourly and daily caps. Transfer allowances are reservations, not provider-enforced usage limits. Missed cron runs or provider failures can leave a box billing beyond its deadline; these controls do not guarantee a hard external bill ceiling.

## Current setup and further reading

Renting is enabled by the deployed vars file, which is kept outside the repository ([deployment guide](DEPLOY.md)); `RENTING_ENABLED=false` in the committed [Worker configuration](wrangler.toml) is only the safe default. `RUN_TOKEN_SECRET` signs run capabilities and `OWNER_STOP_TOKEN` controls emergency cleanup. As of 2026-10-25 acceptance of the current hosted workflow with the protocol-7 payload is still open; historical local smoke results do not establish it.

- [Threat model](docs/THREAT_MODEL.md): who is trusted, and which behaviours are intended design rather than vulnerabilities.
- [Box protocol](docs/BOX_PROTOCOL.md): bootstrap environment, callbacks and result contract.
- [Placement decision](docs/PLACEMENT.md): US/Canada offer preference and the proposed ledger location.
- [Evidence summary](../../traces/vast_broker_summary/README.md): recorded runs, retained evidence and its limitations.
- [Internal implementation and evidence notes](docs/IMPLEMENTATION_NOTES.md): the former README's dated rounds, corrections and superseded behavior.
