# Plan work for a box or fleet

Use `kfci` to turn a versioned run catalog, flat requirements, limits and box snapshots
into plan JSON and per-box executor assignments. The commands read local JSON and print
JSON; they do not start boxes.

From the repository root:

```sh
ci/vast-broker/kfci plan input.json > plan.json
ci/vast-broker/kfci validate input.json plan.json
ci/vast-broker/kfci plan input.json --assignments > assignments.json
ci/vast-broker/kfci plan input.json --without-seeds > cold-plan.json
```

Use `-` instead of `input.json` to read standard input. Input fields and versions are in
`ci/vast-broker/planner/schemas/`. The example at
`ci/vast-broker/planner/fixtures/release-proposed.json` is deliberately proposed, has no
market, and reports unavailable work. Replace its assumptions with reviewed catalog,
requirements, limits and box facts you obtained yourself before execution validation.

For a self-run, supply one box and requirements that fit it. The assignment shape is the
same for an owned workstation, container or provider box. Record host-driver swap, KVM
and VFIO capabilities accurately; a provider name does not grant a capability. Use stable,
provider-qualified physical machine and host IDs.

A valid execution plan must cover every requested run, fit each box's time budget and all
price/cost caps, and use pinned official drivers. Proposed or unpinned inputs fail execution
validation with exit code 2. The offline plan still displays missing work and its quote.

Seeding is optional. An assignment retains its local-build steps and gives each seed hint
a deadline and fallback. `--without-seeds` shows the cold plan with the same requirements.
The assignment-pull route and Python execution are separate integration work; use these
commands to inspect and validate planning data today.
