"""Restore the tested BW6 workspace from tracked seeds; never reset an existing lease."""
from pathlib import Path
import json
import shutil

K = Path(__file__).resolve().parent
D = Path("/workspace/bw6-round-b1r-a1-run")
T = K.parent

def main():
    assert not D.exists(), "workspace exists; never reset it"
    ledger = T / "owned_instances.json"
    if not ledger.exists():
        ledger.write_text("[]\n")
    assert json.loads(ledger.read_text()) == [], "ledger is not empty; never reset it"
    D.mkdir()
    for p in (K / "workspace_seed").iterdir():
        shutil.copy2(p, D / p.name)
    D.joinpath("media").symlink_to("/workspace/bw3-round-b1-run/media", target_is_directory=True)
    D.joinpath("grant_pin.json").write_text('{"machine": null, "variant": "default"}\n')
    D.joinpath("stretch.json").write_text(json.dumps({
        "host_verification": "UNVERIFIED-HOST", "attempts_used": 0,
        "provider_retries_used": 0}) + "\n")
    print("WIRED", D)

if __name__ == "__main__":
    main()
