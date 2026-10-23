"""Capture optional Access headers locally; all transport is fake."""

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[3]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


case = sys.argv[1]
if case == "runner":
    m = load("runner", root / "ci/vast-broker/runner.py")
    pair = {"CF_ACCESS_CLIENT_ID": "runner-a.access", "CF_ACCESS_CLIENT_SECRET": "a" * 64}
    assert m.access_headers(pair) == {
        "CF-Access-Client-Id": "runner-a.access",
        "CF-Access-Client-Secret": "a" * 64,
    }
    assert m.access_headers({}) == {}
    for key in pair:
        try:
            m.access_headers({key: pair[key]})
        except m.Invalid:
            pass
        else:
            raise AssertionError("partial Access pair admitted")
elif case in {"reporter", "daemon"}:
    m = load("reporter", root / "scripts/ci/vast_report.py")
    pair = {"KF_ACCESS_ID": "box-b.access", "KF_ACCESS_SECRET": "b" * 64}
    transport = m.HttpsTransport("mock-run-capability", access=pair)
    if case == "daemon":
        import types

        args = types.SimpleNamespace(
            **{
                key: None
                for key in (
                    "poll flush min_gap retry_base retry_cap rate_backoff final_budget "
                    "max_posts max_bytes line_cap"
                ).split()
            }
        )
        transport = m.make_reporter(
            {
                **pair,
                "KF_RUN_TOKEN": "mock-run-capability",
                "KF_RUN_ID": "123-1",
                "KF_BROKER_URL": "https://broker.example",
            },
            args,
        ).transport

    class Reply:
        status = 200

        def __enter__(self):
            return self

        def __exit__(self, *_):
            pass

        def read(self, _):
            return b"{}"

    class Capture:
        def open(self, request, **_):
            headers = {k.lower(): v for k, v in request.header_items()}
            assert headers["cf-access-client-id"] == pair["KF_ACCESS_ID"]
            assert headers["cf-access-client-secret"] == pair["KF_ACCESS_SECRET"]
            return Reply()

    transport.opener = Capture()
    assert transport.post("https://broker.example/report/123-1", b"{}")[0] == 200
else:
    with tempfile.TemporaryDirectory(prefix="v1-access-headers-") as directory:
        env_file = Path(directory) / "env"
        env_file.write_text(
            "\n".join(
                [
                    "KF_RUN_TOKEN=mock-run-token",
                    "KF_RUN_ID=123-1",
                    "KF_BROKER_URL=https://broker.example",
                    "KF_ACCESS_ID=box-b.access",
                    "KF_ACCESS_SECRET=" + "b" * 64,
                ]
            )
            + "\n"
        )
        curl = Path(directory) / "curl"
        capture = Path(directory) / "capture"
        curl.write_text('#!/bin/sh\ncat > "$CAPTURE"\nprintf 200\n')
        curl.chmod(0o700)
        environment = {
            **os.environ,
            "PATH": directory + ":" + os.environ["PATH"],
            "KF_CI_ENV_FILE": str(env_file),
            "CAPTURE": str(capture),
        }
        for script, args in [
            ("kf-box-kill", []),
            ("kf-box-hold", []),
            ("kf-box-blob-url", ["release-v1", "asset"]),
        ]:
            if {"kill": "kf-box-kill", "hold": "kf-box-hold", "blob": "kf-box-blob-url"}[
                case
            ] != script:
                continue
            result = subprocess.run(
                ["sh", str(root / "scripts/ci" / script), *args],
                env=environment,
                capture_output=True,
                text=True,
            )
            assert result.returncode == 0, (script, result.stderr)
            text = capture.read_text()
            assert "CF-Access-Client-Id: box-b.access" in text, script
            assert "CF-Access-Client-Secret: " + "b" * 64 in text, script
