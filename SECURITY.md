# Security policy

**STATUS: LIVE, 2026-10-23.**

## Reporting a vulnerability

Please report security problems privately, through GitHub's private vulnerability reporting:

<https://github.com/reindertpelsma/kayfabe/security/advisories/new>

Please do not open a public issue for a vulnerability. Include the revision (`KAYFABE_REV.txt` or
the git hash), what you did, and what you saw. Reports are read by the maintainer.

## What counts

kayfabe treats the guest as hostile: guest userspace and guest root are both untrusted, and host
actions are authored from unprivileged verbs, never forwarded from guest bytes (see `AGENTS.md`,
*Rules*). A guest-reachable unbounded read or emit, a guest-controlled host action, or a way for a
guest to reach host memory or devices it was not given is in scope.

## Copyright and licence notices

For a question about a licence or an attribution notice, open an issue in this repository. The
notices we carry are in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and
[`docs/NOTICES.md`](docs/NOTICES.md).
