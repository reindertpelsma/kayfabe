# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Windows llama.cpp output parity against an offline bare-metal reference; never creates a baseline."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from release_oracles import check_llm_run, check_reference, compare_text

PROMPT = "Explain in three sentences why the sky is blue."
SEED, TOKENS, TIMEOUT_S = 1, 64, 480


def file_hash(path):
    with Path(path).open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def main(argv):
    exe, model, refs, out = map(Path, argv)
    try:
        raw = (refs / "llama_cuda.txt").read_bytes()
        meta = json.loads((refs / "llama_cuda.json").read_text(encoding="utf-8-sig"))
        check_reference(meta, dict(prompt=PROMPT, seed=SEED, tokens=TOKENS, temperature=0,
                                  executable_sha256=file_hash(exe), model_sha256=file_hash(model)), raw)
    except OSError as e:
        print("REFERENCE_NOTFOUND", e)
        return 3
    except (ValueError, TypeError) as e:
        print("CHECK llama_reference FAIL", e)
        return 1
    out.mkdir(parents=True, exist_ok=True)
    try:
        p = subprocess.run([str(exe), "-m", str(model), "-p", PROMPT, "-n", str(TOKENS), "-ngl", "99",
                            "--temp", "0", "--seed", str(SEED), "--no-display-prompt"],
                           input=b"", capture_output=True, timeout=TIMEOUT_S, check=False)
        (out / "gen.txt").write_bytes(p.stdout)
        (out / "gen.err").write_bytes(p.stderr)
        log = p.stderr.decode("utf-8", "replace")
        print(log[:60000])
        check_llm_run(p.returncode, log)
        digest = compare_text(p.stdout.decode("utf-8-sig"), raw.decode("utf-8-sig"))
    except (OSError, ValueError, subprocess.TimeoutExpired) as e:
        print("CHECK llama_parity FAIL", e)
        return 1
    print("LLM_PARITY_PASS sha256=" + digest + " seed=1 tokens=64 reference=bare-metal")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
