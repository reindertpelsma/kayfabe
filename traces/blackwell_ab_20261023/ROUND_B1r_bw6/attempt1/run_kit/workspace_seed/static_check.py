"""Dependency-free static checks for the harness, operator and run kit (B1g fix 03:30).

1. compile: every file parses.
2. undefined globals: a name read inside a function that is neither defined at module level,
   imported, a builtin, nor bound locally (symtable). Catches the `new_old` class of defect.
3. module attributes: `alias.attr` chains on locally imported harness modules must resolve.
"""

import ast
import builtins
import importlib
from pathlib import Path
import symtable
import sys
import types

ROOTS = (
    Path("/workspace/bw6-wt/scripts/bench/blackwell"),
    Path("/workspace/bw-provider-retry-wt/scripts/bench/blackwell"),
    Path("/workspace/bw-provider-retry-wt/scripts/bench"),
)
BUILTINS = set(dir(builtins)) | {"__file__", "__name__", "__doc__", "__builtins__", "__spec__"}


def _module_names(tree):
    names = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            for a in node.names:
                names.add((a.asname or a.name).split(".")[0])
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.Global):
            names.update(node.names)
    table = symtable.symtable(ast.unparse(tree), "<module>", "exec")
    names.update(s.get_name() for s in table.get_symbols() if s.is_assigned() or s.is_imported())
    return names, table


def undefined_globals(path):
    source = Path(path).read_text()
    tree = ast.parse(source)
    defined, table = _module_names(tree)
    problems = []

    def visit(t):
        for child in t.get_children():
            if child.get_type() in ("function", "class"):
                for s in child.get_symbols():
                    name = s.get_name()
                    if (s.is_global() or (s.is_free() and child.get_type() == "class")) and (
                        s.is_referenced() and name not in defined and name not in BUILTINS
                    ):
                        problems.append((str(path), child.get_lineno(), "undefined " + name))
            visit(child)

    visit(table)
    return problems


def _local_modules():
    return {p.stem for r in ROOTS for p in r.glob("*.py")}


def attribute_problems(path, extra_paths=()):
    for r in list(extra_paths) + list(reversed(ROOTS)):
        if str(r) not in sys.path:
            sys.path.insert(0, str(r))
    sys.dont_write_bytecode = True
    local = _local_modules() | {p.stem for r in extra_paths for p in Path(r).glob("*.py")}
    tree = ast.parse(Path(path).read_text())
    env, problems = {}, []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for a in node.names:
                if a.name.split(".")[0] in local:
                    try:
                        env[a.asname or a.name] = importlib.import_module(a.name)
                    except Exception as error:  # report, never execute further
                        problems.append((str(path), node.lineno, "import %s: %r" % (a.name, error)))
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1:
            target = node.targets[0]
            chain, value = [], node.value
            while isinstance(value, ast.Attribute):
                chain.append(value.attr)
                value = value.value
            if isinstance(target, ast.Name) and isinstance(value, ast.Name) and value.id in env:
                obj = env[value.id]
                try:
                    for a in reversed(chain):
                        obj = getattr(obj, a)
                except AttributeError:
                    continue
                if isinstance(obj, types.ModuleType):
                    env[target.id] = obj
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and isinstance(node.ctx, ast.Load):
            chain, value = [], node
            while isinstance(value, ast.Attribute):
                chain.append(value.attr)
                value = value.value
            if isinstance(value, ast.Name) and value.id in env:
                obj = env[value.id]
                for a in reversed(chain):
                    if not isinstance(obj, types.ModuleType):
                        break
                    if not hasattr(obj, a):
                        dotted = value.id + "." + ".".join(reversed(chain))
                        problems.append((str(path), node.lineno, "missing attribute " + dotted))
                        break
                    obj = getattr(obj, a)
    return problems


def check(paths, extra_paths=()):
    problems = []
    for path in sorted({str(p) for p in paths}):
        try:
            compile(Path(path).read_text(), path, "exec")
        except SyntaxError as error:
            problems.append((path, error.lineno, "syntax: %s" % error.msg))
            continue
        problems += undefined_globals(path)
        problems += attribute_problems(path, extra_paths)
    return sorted(set(problems))


if __name__ == "__main__":
    found = check(sys.argv[1:])
    for row in found:
        print(*row)
    print("STATIC_CHECK files=%d problems=%d" % (len(set(sys.argv[1:])), len(found)))
    sys.exit(1 if found else 0)
