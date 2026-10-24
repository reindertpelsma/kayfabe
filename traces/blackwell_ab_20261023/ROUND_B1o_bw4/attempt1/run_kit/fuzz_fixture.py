"""Add only an optional arm selector to a disposable native benchmark worktree."""

import argparse
import hashlib
import json
from pathlib import Path

SELECTION = """
    let bw3_selected = std::env::var("KF_BW3_FUZZ_ARM").ok();
    if !matches!(bw3_selected.as_deref(), None | Some("crowd") | Some("crowd-free")) {
        println!("FUZZ_REASON=INVALID_BW3_FIXTURE_SELECTION");
        return false;
    }
"""
FILTER = """
        if bw3_selected.as_deref().is_some_and(|selected| selected != name) {
            continue;
        }
"""


def instrument(source):
    start = source.index("fn concurrent_fuzz(")
    end = source.index("\n/// `cmd[:size]`", start)
    part = source[start:end]
    anchor = "    let crowd_t = (cfg.cores * 3).max(cfg.threads);"
    loop = "    for (name, acfg) in arms {"
    if part.count(anchor) != 1 or part.count(loop) != 1:
        raise ValueError("native fixture arm-loop anchors changed")
    part = part.replace(anchor, SELECTION + anchor).replace(loop, loop + FILTER)
    return source[:start] + part + source[end:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    args = parser.parse_args()
    source = args.source.read_text()
    patched = instrument(source)
    args.source.write_text(patched)
    print(
        json.dumps(
            {
                "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                "fixture_sha256": hashlib.sha256(patched.encode()).hexdigest(),
                "change": "arm selection only; algorithms and default four-arm behavior preserved",
            }
        )
    )


if __name__ == "__main__":
    main()
