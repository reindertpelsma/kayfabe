"""Test process entry: python -m kfml._child CTX.json OUT.json. Writes OUT.json atomically."""
import importlib
import json
import os
import sys
import traceback

from .ctx import Ctx
from .errors import CheckFailed, EnvMissing, NotRun


def _write(path, doc):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(doc, f)
    os.replace(tmp, path)


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    ctx_path, out_path = argv[0], argv[1]
    with open(ctx_path) as f:
        d = json.load(f)
    res = {"status": "FAIL", "reason": "", "metrics": {}}
    try:
        mod = importlib.import_module(d["module"])
        out = mod.run(Ctx(d)) or {}
        res.update(status="PASS", metrics=out.get("metrics", {}), reference=out.get("reference"),
                   dataset_sha256=out.get("dataset_sha256", {}))
    except NotRun as e:
        res.update(status="NOT_RUN", reason=str(e))
    except EnvMissing as e:
        res.update(status="FAIL", reason="env-missing: %s" % e)
    except CheckFailed as e:
        res.update(status="FAIL", reason="check failed: %s" % e)
    except BaseException as e:  # noqa
        tb = traceback.format_exc()
        sys.stderr.write(tb)
        res.update(status="FAIL", reason="%s: %s" % (type(e).__name__, str(e)[:500]))
    _write(out_path, res)
    return 0


if __name__ == "__main__":
    sys.exit(main())
