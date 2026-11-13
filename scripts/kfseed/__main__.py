# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Entry point for `python3 -m kfseed` and for `python3 -I /abs/path/kfseed/__main__.py`.

Run by path, nothing is taken from the cwd or sys.path: the package is loaded from this file's own directory.
"""
import importlib.util
import os
import sys

if not __package__:
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("kfseed", os.path.join(here, "__init__.py"),
                                                  submodule_search_locations=[here])
    pkg = importlib.util.module_from_spec(spec)
    sys.modules["kfseed"] = pkg
    spec.loader.exec_module(pkg)
    from kfseed.cli import main
else:
    from .cli import main

sys.exit(main())
