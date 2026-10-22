"""Fake process/clock/curl; never touches a GPU or network."""

import builtins, contextlib, io, json, os, select, subprocess, sys, tempfile, types

code = sys.argv[1]
exhausted = sys.argv[2]


class Pipe:
    def fileno(self):
        return 99


class Child:
    stdin = io.BytesIO()
    stdout = Pipe()

    def wait(self):
        return 0


calls = []
ticks = [0]
reads = [0]
original_open = builtins.open
with tempfile.TemporaryDirectory(prefix="v5-sender-") as directory:

    def sandbox(path, *args, **kwargs):
        if str(path).startswith("/workspace/"):
            path = directory + "/" + str(path).rsplit("/", 1)[1]
        return original_open(path, *args, **kwargs)

    builtins.open = sandbox
    subprocess.Popen = lambda *_args, **_kwargs: Child()
    subprocess.run = lambda *_args, **kwargs: (
        calls.append(json.loads(kwargs["input"]))
        or types.SimpleNamespace(returncode=0, stdout=b"{}")
    )
    select.select = lambda readable, *_: (readable, [], [])

    def read(*_):
        reads[0] += 1
        return b"payload output\n" * 500 if reads[0] <= 100 else b""

    os.read = read
    import time

    def monotonic():
        ticks[0] += 10
        return ticks[0]

    time.monotonic = monotonic
    code = (
        code.replace("attempted=0", "attempted=600")
        if exhausted == "attempts"
        else code.replace("sent=0", "sent=900000")
    )
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            exec(code, {})
    except SystemExit:
        pass
    assert any(
        call["kind"] == "log" and call.get("text") == "" for call in calls
    ), "busy log branch starved bounded heartbeat"
