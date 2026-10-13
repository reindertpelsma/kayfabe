# Loose host helper scripts, 2026-10-13

**STATUS: ARCHIVE (not product code).** Copies of helper scripts that lived only in `/tmp` and `/data` on the trusted host `root@172.22.1.20` (the per-run cleanup,
capture, staging and campaign helpers of the Windows USER-ring work and of the 2026-10-13 default-route validation), saved because they were not in git. Only files changed in the
last 3 days and not already present in the repository (by git blob hash) are included; a pattern scan for credentials found none. They carry hard-coded run numbers, PIDs and paths and are
a record of what ran, not scripts to replay unchanged. The Windows guest password is read at run time from the host's secrets file and is not in any of these files.
