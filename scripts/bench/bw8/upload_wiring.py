"""Relocate inherited upload bindings structurally, preserving its real upload protocol."""

import ast
from pathlib import Path


def single(nodes, label):
    if len(nodes) != 1:
        raise ValueError("missing or duplicate upload anchor: " + label)
    return nodes[0]


def path_value(value):
    path = Path(value)
    if not path.is_absolute():
        raise ValueError("operator upload paths must be absolute")
    return ast.Call(
        func=ast.Name(id="Path", ctx=ast.Load()), args=[ast.Constant(str(path))], keywords=[]
    )


def relocate_upload(text, *, D, W, O, K, probes, ledger, extra_files):
    tree = ast.parse(text)
    for name, value in dict(D=D, W=W, O=O, K=K, probes=probes).items():
        matches = [
            node
            for node in tree.body
            if isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == name
        ]
        single(matches, name).value = path_value(value)
    reads = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == "read_text"
        and any(
            isinstance(child, ast.Constant)
            and isinstance(child.value, str)
            and child.value.endswith("owned_instances.json")
            for child in ast.walk(node.func.value)
        )
    ]
    read = single(reads, "current ledger")
    read.func.value = path_value(ledger)
    bundles = [
        node
        for node in tree.body
        if isinstance(node, ast.Assign)
        and any(isinstance(target, ast.Name) and target.id == "bundle" for target in node.targets)
    ]
    bundle = single(bundles, "bundle assembly")
    extras = ast.Dict(
        keys=[ast.Constant(key) for key in extra_files],
        values=[path_value(value) for value in extra_files.values()],
    )
    update = ast.Expr(
        value=ast.Call(
            func=ast.Attribute(
                value=ast.Name(id="files", ctx=ast.Load()), attr="update", ctx=ast.Load()
            ),
            args=[extras],
            keywords=[],
        )
    )
    tree.body.insert(tree.body.index(bundle), update)
    ast.fix_missing_locations(tree)
    return ast.unparse(tree) + "\n"
