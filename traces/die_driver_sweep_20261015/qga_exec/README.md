# Windows guest-agent command result recovery

STATUS: LIVE, 2026-10-15. Harness correction; not a GPU Code43 fix.

Measured GB206 Windows wrapper52404 exited1 at a guest-agent receive timeout,
then a separate call recovered the actual failed-start adapter evidence. Source
c2092cf7's client abandoned the already launched command on a status timeout.

Hypothesis before the fix: reconnecting and polling that same accepted PID can
recover its result without replaying a command with side effects. Falsifier:
multiple guest-exec requests, a changed PID, a hidden nonzero exit code or an
unbounded reconnect. Test-first10339185b: four tests, three expected errors;
before.txt retains the output. The launch-reply-loss test already passes: an
unknown launch outcome must not be replayed.

The correction reconnects only during status polling, resynchronizes the new
socket, preserves the original PID/output/exit code, and bounds recovery by a
monotonic deadline. An agent protocol error fails immediately; deadline returns
124. Lost launch replies still raise an error, without resubmitting. Connection
and sync failures close their sockets. No guest command is inferred successful.

After: all34 bench harness tests pass, including six command-result tests and a
real AF_UNIX agent fixture that closes the first status connection, accepts one
new connection and returns exit7 for the original PID. Only one guest-exec is
observed. after.txt records the run. Windows hardware transport recovery is not
claimed; the confirmed GB206 GPU initialization failure remains open.
