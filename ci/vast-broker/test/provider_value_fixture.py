"""Seeded semantic privacy checks; no network or provider access."""

import ast
import base64
import contextlib
import io
import ipaddress
import json
import re
import runpy
import sys

x = json.load(sys.stdin)
r = runpy.run_path(sys.argv[1], run_name="value_fixture")
g = {"re": re, "ipaddress": ipaddress, "base64": base64, "json": json}
exec(
    compile(
        ast.Module(
            body=[n for n in ast.parse(x["renderer"]).body if isinstance(n, ast.FunctionDef)],
            type_ignores=[],
        ),
        "literal_renderer",
        "exec",
    ),
    g,
)
for row in x["rows"]:
    context = row["context"]
    secret = "leftprivate" + str(row["port"]) + "rightprivate"
    for text in [secret, secret[:4] + "\x01" + secret[4:]]:
        safe = r["sanitise"](text, [secret], context)[0]
        assert "leftprivate" not in safe and "rightprivate" not in safe, safe
    g["data"] = {"endpoint_redactions": base64.b64encode(json.dumps(context).encode()).decode()}
    stream, summary = [], io.StringIO()
    runner = r["Runner"](
        {"endpoint_redactions": context}, {}, None, emit=stream.append, summary=summary
    )
    runner.show(row["raw"])
    runner.raw = [row["raw"]]
    runner.emit(row["raw"])
    runner.summary_write(row["raw"])
    rendered = io.StringIO()
    with contextlib.redirect_stdout(rendered):
        g["public_print"](row["raw"])
    for output in [
        r["redact_endpoint_values"](row["raw"], context),
        r["sanitise"](row["raw"], (), context)[0],
        "".join(stream),
        runner.sanitised()[0],
        summary.getvalue(),
        rendered.getvalue(),
        g["redact_endpoint_values"](row["raw"], context),
        g["clean"](row["raw"]),
    ]:
        for value in [row["ip"], row["alias"], row["ipv6"]]:
            assert value.lower() not in output.lower(), (value, output)
        assert not any(int(m[0]) == row["port"] for m in re.finditer(r"[0-9]+", output)), output
        assert "count=7 driver=580.88" in output, output
print("512 seeded command/log shapes passed all Python output boundaries")

for malformed in [
    None,
    "!",
    "A" * 16385,
    base64.b64encode(b"{}").decode(),
    base64.b64encode(b'{"ips":[],"ports":[true],"overflow":false}').decode(),
]:
    assert r["decode_endpoint_values"](malformed) is None
    assert g["decode_endpoint_values"](malformed) is None

for helper in [r["redact_endpoint_values"], g["redact_endpoint_values"]]:
    assert helper("hex=deadbeef", {"ips": [], "ports": [], "overflow": True}) == "hex=[redacted]"
