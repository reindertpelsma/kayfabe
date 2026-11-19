# Token check: per-image checklist (TEMPLATE)

STATUS: LIVE, 2026-11-18 (template; copy it, fill in the placeholders for ONE sealed image, keep the copy private). Used with
`scripts/bench/windows/propimage_token_check.py ... --checklist YOUR_COPY.md`. The tool refuses a checklist that has no
SERVER ROUND-TRIP section and no PASS and FAIL texts (the lesson below).

Hosts: <vendor-host-1>, <vendor-host-2>, <vendor-host-3>

The `Hosts:` line (optional, comma separated, host names only) is the list of identity-server hosts that the indicator report probes with a
TCP 443 connect (reachability only, no request). `--hosts` on the command line overrides it.

## What the check emulates

An untrusted party holds a copy of the sealed image and boots a throwaway overlay of it online, with the real clock. The question is whether any
cached login in the image is still valid on the vendor's server. Nothing is typed into the guest except to see whether a prompt appears (then
cancel). Read the printed INDICATOR REPORT first (SUMMARY lines at its end, and "Sign-on file mtimes": a sign-on file touched at this boot's time
means the app talked to its identity server). The report prints only booleans, counts, file names, sizes and modification times, never values.

## The lesson, MEASURED on 2026-11-18

"Shows signed in" is NOT proof. Several apps start in an offline mode (no server contact at launch) or show a cached identity and a cached,
offline licence lease, with or without a valid server session. On 2026-11-18, one app looked signed in while a decisive action had not yet
been tried, and a later server round-trip action (creating a cloud share link) showed that the cached session was STILL valid server-side. Only
an action that needs a valid server token counts. A "looks signed out" or "looks signed in" observation (part A) may only ever support a verdict;
it never decides one.

## A. Look (cheap, weak: never a verdict by itself)

For each app: open it, note the account shown (yes or no, never write the name down), the licence type shown (named user, education, key,
trial, none), and whether it asks to sign in.

## B. SERVER ROUND-TRIP ACTIONS (the real test)

Write one entry per app that has an account. Each entry names ONE action that cannot succeed without a valid server token, and its expected texts.

1. Steam (if installed): it may start in OFFLINE mode (no server contact). Menu Steam > "Go Online".
   PASS: asks for the password, shows a sign-in dialog, or "cannot connect" with the connection log showing the server's `Access Denied` for the cached
   logon (the indicator report prints those outcome lines, masked). FAIL: goes online and shows friends or the store as an account.
2. <Vendor with a cloud> (<app>): <the cloud action, e.g. create a share link for a document, open a cloud file, refresh the entitlement>.
   PASS: asks to sign in again, says the session expired, or fails. FAIL: the action succeeds (a link is created, cloud files are listed, the
   entitlement refreshes).
3. <Vendor with a portal> (<app>): <the portal action, e.g. open the portal list and load "my content">.
   PASS: not signed in, or asks for credentials. FAIL: the portal is listed as signed in and content loads, or a named-user licence refreshes.
4. <Another vendor account action>: <action>. PASS: <text>. FAIL: <text>.
5. Browser: signed out is PASS. A signed-in profile that loads synced data is FAIL.
6. After B run `report` again and compare the sign-on file mtimes with the first report.

A licence KEY (serial, node lock) is a licence fact, not a login: write it down, it is not a FAIL. An account shown, an automatic login, or a
server action that succeeds is a FAIL.

## Verdict

PASS overall: every round-trip action in B asks to sign in again, fails, or says not signed in.
FAIL overall: any round-trip action succeeds as an account, or an app goes online without a password.
UNDECIDED: part A only, or a round-trip action that could not be tried. Treat as FAIL until it is tried.

## On FAIL (vendor side, from a trusted machine; signing out inside the guest does NOT revoke a stored token)

For each failing vendor: change the account password and end all sessions, deauthorize other devices or deregister the device or key, and have an
administrator revoke tokens or named-user licences where the vendor has that. Then run the check again on a FRESH overlay (a new `prepare`) with the
decisive round-trip action, and record each run's result (private notes, without account names or e-mail addresses).

When done: `end` (clean shutdown, the overlay and all copies are deleted). Never leave the window open overnight.
