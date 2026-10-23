"""Label each exported artifact; retain source bytes through the existing source-hash receipts."""

from pathlib import Path
import hashlib, json

LABEL = "UNVERIFIED-HOST"


def text_label(name, text):
    if name.endswith((".json", ".jsonl")):
        try:
            if name.endswith(".jsonl"):
                rows = [json.loads(line) for line in text.splitlines() if line.strip()]
                if not rows:
                    return (
                        json.dumps(
                            {
                                "experiment_mode": "OBSERVATION",
                                "attestation": False,
                                "host_verification": LABEL,
                                "record_type": "empty_source_stream",
                                "original_rows": 0,
                            }
                        )
                        + "\n"
                    )
                if all(isinstance(row, dict) for row in rows):
                    return "".join(
                        json.dumps(
                            {
                                **row,
                                "host_verification": LABEL,
                                "experiment_mode": "OBSERVATION",
                                "attestation": False,
                            }
                        )
                        + "\n"
                        for row in rows
                    )
            value = json.loads(text)
            if isinstance(value, dict):
                value["host_verification"] = LABEL
                value["experiment_mode"] = "OBSERVATION"
                value["attestation"] = False
                return json.dumps(value, indent=2) + "\n"
            if isinstance(value, list):
                if value and all(isinstance(row, dict) for row in value):
                    for row in value:
                        row.update(
                            host_verification=LABEL,
                            experiment_mode="OBSERVATION",
                            attestation=False,
                        )
                    return json.dumps(value, indent=2) + "\n"
                return (
                    json.dumps(
                        {
                            "host_verification": LABEL,
                            "experiment_mode": "OBSERVATION",
                            "attestation": False,
                            "records": value,
                        },
                        indent=2,
                    )
                    + "\n"
                )
        except (ValueError, TypeError):
            pass
    return (
        text
        if text.startswith(LABEL + "\nOBSERVATION")
        else LABEL + "\nOBSERVATION; not PASS or attestation\n" + text
    )


def label_exports(root):
    root = Path(root)
    receipts = []
    for p in root.rglob("*"):
        if not p.is_file():
            continue
        if p.name == "export_receipt.json":
            receipts.append(p)
            continue
        assert p.suffix.lower() not in (".png", ".dmp", ".exe", ".dll", ".iso", ".qcow2")
        text = p.read_text()
        p.write_text(text_label(p.name, text))
    for p in receipts:
        receipt = json.loads(p.read_text())
        if not receipt:
            receipt = {"_evidence_context": {"host_verification": LABEL, "source_files": 0}}
        for name, row in receipt.items():
            if not isinstance(row, dict):
                continue
            target = p.parent / name
            if target.is_file():
                row["export_sha256"] = hashlib.sha256(target.read_bytes()).hexdigest()
            row["host_verification"] = LABEL
            row["experiment_mode"] = "OBSERVATION"
            row["attestation"] = False
            row["normalization"] = (
                "UNVERIFIED-HOST label plus existing whitespace/UUID normalization"
            )
        p.write_text(json.dumps(receipt, indent=2) + "\n")
    assert all(LABEL.encode() in p.read_bytes() for p in root.rglob("*") if p.is_file())


if __name__ == "__main__":
    import sys

    label_exports(sys.argv[1])
