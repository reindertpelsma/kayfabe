# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""G5, the export table (docs/design/V3_SEC_PERIMETER.md §3; OWNER_RULINGS §R gate 3).

Every item a perimeter file exports to safe code is a row of docs/design/PERIMETER_EXPORTS.md,
with the checks it performs and the test that shows each check. The inventory comes from the
compiler's own item graph (rustdoc JSON, after cfg and macro expansion, private and hidden
items included), unioned over the target matrix; the table must equal it.

Rules: E1 (inventory equals the table; E1b the reach check; E1c the cfg rule), E2 (mechanical
columns), E3-E3e (an OK row's checks each name a test that runs in CI and names the item),
E4 (# Safety), E5-E8 (handle and view types), E9 (safe extern "C" fns taking a raw pointer:
an exact baseline that only goes down), E10 (dated reasons), E11 (validation dependencies),
E12 (generic exports), E13 (safe traits), E14 (statics). E3-E9 and E12-E14 bind OK rows; an
OPEN or LANE row records its violation in its reason.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath

import rslex

TABLE = "docs/design/PERIMETER_EXPORTS.md"
HEADER = "| item | vis | kind | checks | tests | status |"
SEP = "|---|---|---|---|---|---|"
VD_HEADER = "| file | role | why |"
VD_SEP = "|---|---|---|"
TYPE_KINDS = ("owning handle", "borrowed view", "FFI struct", "plain data")
MECH_KINDS = ("safe fn", "unsafe fn", "safe extern fn", "unsafe extern fn", "trait", "unsafe trait",
              "trait impl", "unsafe trait impl", "type alias", "static", "const", "macro", "auto trait")
# Derived impls that are not rows. `TrivialClone` is the std-internal marker rustc 1.99's
# `derive(Clone)` emits beside `Clone` (doc(hidden) in core, so it appears only with
# --document-hidden-items: measured in CI run 37165994390, 2026-10-04, 24 such impls).
EXCLUDED_DERIVES = {"Debug", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "StructuralPartialEq", "TrivialClone"}
DATE = r"\d{4}-\d{2}-\d{2}"
STATUS_RE = re.compile(rf"^(OK|OPEN: {DATE}: .+|LANE:[\w.-]+: {DATE}: .+)$")
REF_RE = re.compile(r"^(t|ui|box|equiv):(.+)$")


@dataclass
class Row:
    file: str
    item: str
    vis: str
    kind: str
    checks: str = ""
    tests: str = ""
    status: str = ""
    line: int = 0
    meta: dict = field(default_factory=dict)

    def cells(self) -> list[str]:
        return [f"`{self.item}`", self.vis, self.kind, self.checks, self.tests, self.status]


@dataclass
class Finding:
    rule: str
    path: str
    line: int
    msg: str

    def __str__(self) -> str:
        return f"{self.rule} {self.path}:{self.line}: {self.msg}"

    def __lt__(self, other: "Finding") -> bool:
        return (self.rule, self.path, self.line, self.msg) < (other.rule, other.path, other.line, other.msg)


# ---------------------------------------------------------------------------------------
# rustdoc JSON
# ---------------------------------------------------------------------------------------


# What the generator's cargo inherits: nothing else (G1's allowlist, compiler_location.sh step 1).
RUSTDOC_ENV_KEEP = ("PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME", "CARGO_TERM_COLOR")


def rustdoc_env(out: Path, toolchain: str | None) -> dict[str, str]:
    """The generator's environment, from an allowlist. The pinned toolchain's own `rustc` and
    `rustdoc` are named explicitly (`RUSTDOC`/`RUSTC` override `build.rustdoc`/`build.rustc` in any
    cargo config), and both wrappers are set EMPTY, which disables a configured one. ⊘ The first
    version inherited the whole environment: a committed `[build] rustdoc = "./rd.sh"` ran in
    its place and could filter the JSON this table is built from (review 2026-10-04)."""
    env = {k: os.environ[k] for k in RUSTDOC_ENV_KEEP if k in os.environ}
    if toolchain:
        env["RUSTUP_TOOLCHAIN"] = toolchain
        for tool, var in (("rustc", "RUSTC"), ("rustdoc", "RUSTDOC")):
            r = subprocess.run(["rustup", "which", tool, "--toolchain", toolchain], capture_output=True, text=True,
                               env=env)
            if r.returncode != 0 or not r.stdout.strip():
                raise SystemExit(f"rustup cannot find {tool} for the pinned toolchain {toolchain}: {r.stderr}")
            env[var] = r.stdout.strip()
    env.update({"RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "", "RUSTC_BOOTSTRAP": "1",
                "CARGO_TARGET_DIR": str(out)})
    return env


def run_rustdoc(root: Path, crate: str, target: str, out: Path, toolchain: str | None = None) -> Path:
    """`cargo rustdoc --output-format json`, private and hidden items included, in `rustdoc_env`.
    RUSTC_BOOTSTRAP is set for THIS process only (never in a workflow env); the target dir is
    separate and no verdict uses its build."""
    env = rustdoc_env(out, toolchain)
    cmd = ["cargo", "rustdoc", "-p", crate, "--lib", "--locked", "--target", target, "--",
           "-Z", "unstable-options", "--output-format", "json", "--document-private-items",
           "--document-hidden-items"]
    r = subprocess.run(cmd, cwd=root, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"rustdoc failed for {crate} ({target}):\n{r.stderr[-4000:]}")
    return out / target / "doc" / f"{crate.replace('-', '_')}.json"


class Doc:
    def __init__(self, data: dict, fmt: int):
        if data.get("format_version") != fmt:
            raise SystemExit(f"EF10: rustdoc JSON format_version {data.get('format_version')} != pinned {fmt} "
                             "(perimeter.toml rustdoc_format): re-check the generator before bumping")
        self.data = data
        self.idx = data["index"]

    def get(self, i) -> dict:
        return self.idx[str(i)]

    @staticmethod
    def file(it: dict) -> str | None:
        sp = it.get("span")
        return sp["filename"] if sp else None

    @staticmethod
    def kind(it: dict) -> str:
        return next(iter(it["inner"]))


def render_type(t, depth: int = 0) -> str:
    if depth > 8 or t is None:
        return "…"
    if not isinstance(t, dict):
        return str(t)
    k, v = next(iter(t.items()))
    if k == "resolved_path":
        name = v["path"].split("::")[-1]
        args = v.get("args") or {}
        ab = args.get("angle_bracketed") if isinstance(args, dict) else None
        if ab and ab.get("args"):
            inner = [render_type(a["type"], depth + 1) if "type" in a else
                     (a.get("lifetime") or "…") for a in ab["args"]]
            inner = [x for x in inner if not x.startswith("'")]
            if inner:
                return f"{name}<{', '.join(inner)}>"
        return name
    if k == "primitive":
        return v
    if k == "generic":
        return v
    if k == "borrowed_ref":
        return f"&{'mut ' if v.get('is_mutable') else ''}{render_type(v['type'], depth + 1)}"
    if k == "raw_pointer":
        return f"*{'mut' if v.get('is_mutable') else 'const'} {render_type(v['type'], depth + 1)}"
    if k == "slice":
        return f"[{render_type(v, depth + 1)}]"
    if k == "array":
        return f"[{render_type(v['type'], depth + 1)}; {v.get('len')}]"
    if k == "tuple":
        return "(" + ", ".join(render_type(x, depth + 1) for x in v) + ")"
    if k == "function_pointer":
        return "fn(…)"
    if k == "dyn_trait":
        return "dyn " + " + ".join(x["trait"]["path"].split("::")[-1] for x in v.get("traits", []))
    if k == "impl_trait":
        return "impl …"
    if k == "qualified_path":
        return v.get("name", "…")
    return k


def type_contains(t, pred, depth: int = 0) -> bool:
    if depth > 12 or t is None:
        return False
    if pred(t):
        return True
    if isinstance(t, dict):
        return any(type_contains(v, pred, depth + 1) for v in t.values())
    if isinstance(t, list):
        return any(type_contains(v, pred, depth + 1) for v in t)
    return False


def vis_text(v) -> str:
    if v == "public":
        return "pub"
    if v == "crate":
        return "pub(crate)"
    if v == "default":
        return "default"
    if isinstance(v, dict) and "restricted" in v:
        return f"pub(in {v['restricted'].get('path', '?')})"
    return str(v)


def fn_kind(it: dict) -> str:
    h = it["inner"]["function"]["header"]
    abi = h.get("abi")
    ext = not (abi == "Rust" or (isinstance(abi, str) and abi == "Rust"))
    return f"{'unsafe' if h.get('is_unsafe') else 'safe'} {'extern fn' if ext else 'fn'}"


# ---------------------------------------------------------------------------------------
# the inventory
# ---------------------------------------------------------------------------------------


@dataclass
class TypeInfo:
    name: str
    file: str
    vis: str
    fields: list  # (name, vis, type)
    has_lifetime: bool
    repr_c: bool
    traits: set  # explicit (non-synthetic) trait impls by name
    auto: set  # synthetic positive auto-trait impls by name
    is_enum: bool


def unsafe_impl_lines(root: Path, files: set[str]) -> dict[str, set[int]]:
    out: dict[str, set[int]] = {}
    for f in files:
        p = root / f
        if not p.exists():
            continue
        s = rslex.Structure(rslex.tokenize_file(p, f), f)
        out[f] = {x.line for x in rslex.unsafe_sites(s) if x.kind in ("unsafe_impl", "unsafe_trait")}
    return out


def inventory(doc: Doc, perimeter: set[str], unsafe_lines: dict[str, set[int]]) -> tuple[dict, dict, dict]:
    """Rows keyed (file, item); types keyed (file, name); and every item name per file (any
    visibility) for the reach check. Also `names[("impls", f)]`: the (Self, Trait) pairs of the
    trait impls whose span is in `f`, derived ones included (E1b's impl half), and
    `names[("spliced", f)]`: items whose span is in perimeter file `f` but whose module is
    another file's (E1d)."""
    rows: dict[tuple[str, str], Row] = {}
    types: dict[tuple[str, str], TypeInfo] = {}
    names: dict = {}
    idx = doc.idx

    def module_file(mid) -> str | None:
        m = idx.get(str(mid))
        return Doc.file(m) if m else None

    def exported(it: dict, f: str) -> bool:
        v = it.get("visibility")
        if v in ("public", "crate"):
            return True
        if isinstance(v, dict) and "restricted" in v:
            return module_file(v["restricted"].get("parent")) != f
        return False

    def add(f: str, item: str, vis: str, kind: str, meta: dict | None = None) -> None:
        rows.setdefault((f, item), Row(f, item, vis, kind, meta=meta or {}))

    def note(f: str, name: str | None) -> None:
        if name:
            names.setdefault(f, set()).add(name)

    # ★ Exported macros, by their SPAN, whatever module lists them: rustdoc puts every
    # `#[macro_export]` macro under the crate ROOT module (measured 2026-10-04: perimeter module
    # items [], root items [the macros]), so a walk of perimeter modules never saw one, and the
    # `macro` row kind could not fire (review 2026-10-04).
    for it in idx.values():
        if Doc.kind(it) == "macro" and Doc.file(it) in perimeter:
            f = Doc.file(it)
            note(f, it.get("name"))
            if exported(it, f):
                add(f, f"{it['name']}!", vis_text(it["visibility"]), "macro", {"id": it["id"]})
    # E1d: an item whose span is a perimeter file inside a module that is NOT that file's is
    # spliced code (`include!`, an alias of it, a macro): the compiler attributes it to the
    # perimeter file, so only this can see where it landed (review 2026-10-04).
    for it in idx.values():
        if Doc.kind(it) != "module" or Doc.file(it) is None:
            continue
        for cid in it["inner"]["module"]["items"]:
            c = idx.get(str(cid))
            if c is None or Doc.kind(c) in ("macro", "use", "module"):  # a module's span is its own file
                continue
            cf = Doc.file(c)
            if cf in perimeter and cf != Doc.file(it):
                names.setdefault(("spliced", cf), set()).add(f"{c.get('name')} (in the module of {Doc.file(it)})")
    for it in idx.values():
        if Doc.kind(it) != "module" or Doc.file(it) not in perimeter:
            continue
        mf = Doc.file(it)
        for cid in it["inner"]["module"]["items"]:
            c = doc.get(cid)
            k = Doc.kind(c)
            f = Doc.file(c) or mf
            if f not in perimeter:
                continue
            note(f, c.get("name"))
            ex = exported(c, f)
            if k == "function":
                if ex:
                    add(f, c["name"], vis_text(c["visibility"]), fn_kind(c), {"id": c["id"], "fn": c})
            elif k in ("struct", "enum", "union"):
                types[(f, c["name"])] = type_info(doc, c, f)
                if ex:
                    add(f, c["name"], vis_text(c["visibility"]), "type", {"id": c["id"]})
            elif k == "trait":
                line = c["span"]["begin"][0]
                tk = "unsafe trait" if c["inner"]["trait"].get("is_unsafe") or line in unsafe_lines.get(f, ()) \
                    else "trait"
                if ex:
                    add(f, f"trait {c['name']}", vis_text(c["visibility"]), tk, {"id": c["id"], "trait": c})
                for mid in c["inner"]["trait"]["items"]:
                    m = doc.get(mid)
                    note(f, m.get("name"))
                    if ex and Doc.kind(m) == "function":
                        add(f, f"{c['name']}::{m['name']}", vis_text(c["visibility"]), fn_kind(m),
                            {"id": m["id"], "fn": m, "trait_def": True})
            elif k == "type_alias":
                if ex:
                    add(f, c["name"], vis_text(c["visibility"]), "type alias",
                        {"id": c["id"], "alias": c["inner"]["type_alias"]["type"]})
            elif k == "constant":
                if ex:
                    add(f, c["name"], vis_text(c["visibility"]), "const", {"id": c["id"]})
            elif k == "static":
                if ex:
                    add(f, c["name"], vis_text(c["visibility"]), "static",
                        {"id": c["id"], "static": c["inner"]["static"]})
            elif k == "macro":
                pass  # rows for every macro by its span, above
    # impls: membership by span, whatever the Self type
    for it in idx.values():
        if Doc.kind(it) != "impl" or Doc.file(it) not in perimeter:
            continue
        imp = it["inner"]["impl"]
        if imp.get("is_synthetic") or imp.get("blanket_impl") is not None:
            continue
        f = Doc.file(it)
        for mid in imp["items"]:  # every item name, for the reach check, before any filter
            note(f, idx[str(mid)].get("name"))
        self_t = imp["for"]
        self_name = render_type(self_t)
        if imp.get("trait") is not None:
            names.setdefault(("impls", f), set()).add((impl_key(self_name), imp["trait"]["path"].split("::")[-1]))
        local = self_t.get("resolved_path", {}).get("id") if isinstance(self_t, dict) else None
        local_item = idx.get(str(local)) if local is not None else None
        if local_item is not None and Doc.file(local_item) is not None:
            self_exported = exported(local_item, Doc.file(local_item))
        else:
            self_exported = True  # foreign or generic Self: reachable from anywhere
        tr = imp.get("trait")
        if tr is not None:
            tname = render_type({"resolved_path": tr})
            base = tr["path"].split("::")[-1]
            if base in EXCLUDED_DERIVES and "automatically_derived" in it.get("attrs", []):
                continue
            if not self_exported:
                continue
            line = it["span"]["begin"][0]
            kind = "unsafe trait impl" if imp.get("is_unsafe") or line in unsafe_lines.get(f, ()) else "trait impl"
            add(f, f"<{self_name} as {tname}>", "default", kind, {"id": it["id"], "impl": it, "trait": base})
            for mid in imp["items"]:
                m = doc.get(mid)
                note(f, m.get("name"))
                if Doc.kind(m) == "function":
                    add(f, f"<{self_name} as {tname}>::{m['name']}", "default", fn_kind(m),
                        {"id": m["id"], "fn": m, "trait_impl": base})
        else:
            for mid in imp["items"]:
                m = doc.get(mid)
                note(f, m.get("name"))
                if not exported(m, f):
                    continue
                k = Doc.kind(m)
                if k == "function":
                    add(f, f"{self_name}::{m['name']}", vis_text(m["visibility"]), fn_kind(m), {"id": m["id"], "fn": m})
                elif k == "assoc_const":
                    add(f, f"{self_name}::{m['name']}", vis_text(m["visibility"]), "const", {"id": m["id"]})
    return rows, types, names


def type_info(doc: Doc, it: dict, f: str) -> TypeInfo:
    k = Doc.kind(it)
    inner = it["inner"][k]
    fields = []
    if k == "struct":
        kind = inner["kind"]
        ids = []
        if isinstance(kind, dict) and "plain" in kind:
            ids = kind["plain"]["fields"]
        elif isinstance(kind, dict) and "tuple" in kind:
            ids = [x for x in kind["tuple"] if x is not None]
        for fid in ids:
            fd = doc.get(fid)
            fields.append((fd["name"], fd["visibility"], fd["inner"]["struct_field"]))
    elif k == "union":
        for fid in inner.get("fields", []):
            fd = doc.get(fid)
            fields.append((fd["name"], fd["visibility"], fd["inner"]["struct_field"]))
    elif k == "enum":
        for vid in inner.get("variants", []):
            var = doc.get(vid)
            vk = var["inner"]["variant"]["kind"]
            ids = []
            if isinstance(vk, dict) and "struct" in vk:
                ids = vk["struct"]["fields"]
            elif isinstance(vk, dict) and "tuple" in vk:
                ids = [x for x in vk["tuple"] if x is not None]
            for fid in ids:
                fd = doc.get(fid)
                # a variant field is public by construction: it takes the ENUM's visibility
                fields.append((fd["name"], it["visibility"], fd["inner"]["struct_field"]))
    traits, auto = set(), set()
    for iid in inner.get("impls", []):
        im = doc.idx.get(str(iid))
        if not im:
            continue
        ii = im["inner"]["impl"]
        if ii.get("trait") is None or ii.get("is_negative"):
            continue
        name = ii["trait"]["path"].split("::")[-1]
        if ii.get("is_synthetic"):
            auto.add(name)
        elif ii.get("blanket_impl") is None:
            traits.add(name)
    gens = inner.get("generics", {}).get("params", [])
    has_lt = any("lifetime" in p.get("kind", {}) for p in gens)
    repr_c = any(isinstance(a, dict) and a.get("repr", {}).get("kind") == "c" for a in it.get("attrs", []))
    return TypeInfo(it["name"], f, vis_text(it["visibility"]), fields, has_lt, repr_c, traits, auto, k == "enum")


INT_PRIMS = {"u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize"}


def address_carrying(types: dict[tuple[str, str], TypeInfo]) -> set[str]:
    """E6, to a fixpoint over local types."""
    by_name = {t.name: t for t in types.values()}
    carrying: set[str] = set()
    changed = True
    while changed:
        changed = False
        for t in by_name.values():
            if t.name in carrying:
                continue
            hit = False
            for fname, _, fty in t.fields:
                if type_contains(fty, lambda x: isinstance(x, dict) and ("raw_pointer" in x or "function_pointer" in x)):
                    hit = True
                elif type_contains(fty, lambda x: isinstance(x, dict) and "resolved_path" in x and (
                        x["resolved_path"]["path"].split("::")[-1] == "NonNull"
                        or x["resolved_path"]["path"].split("::")[-1] in carrying)):
                    hit = True
                elif isinstance(fty, dict) and fty.get("primitive") in INT_PRIMS and re.fullmatch(
                        r"ptr|addr|base|\w+_ptr|\w+_addr", fname or ""):
                    hit = True
            if (not hit and len(t.fields) == 1 and t.fields[0][0] == "0"
                    and isinstance(t.fields[0][2], dict) and t.fields[0][2].get("primitive") in INT_PRIMS
                    and re.search(r"Ptr|Addr|Span|Handle|Base", t.name)):
                hit = True
            if hit:
                carrying.add(t.name)
                changed = True
    return carrying


def fd_carrying(types: dict[tuple[str, str], TypeInfo]) -> set[str]:
    """E7."""
    out = set()
    for t in types.values():
        for fname, _, fty in t.fields:
            if type_contains(fty, lambda x: isinstance(x, dict) and "resolved_path" in x and
                             x["resolved_path"]["path"].split("::")[-1] in ("OwnedFd", "BorrowedFd", "RawFd")):
                out.add(t.name)
            elif re.fullmatch(r"fd|\w+_fd", fname or ""):
                out.add(t.name)
            elif t.name.endswith("Fd") and isinstance(fty, dict) and fty.get("primitive") == "i32":
                out.add(t.name)
    return out


def auto_rows(types: dict, table_kinds: dict[tuple[str, str], str], carrying: set[str], fds: set[str]) -> dict:
    rows = {}
    for (f, name), t in types.items():
        kind = table_kinds.get((f, name))
        if kind in ("owning handle", "borrowed view") or name in carrying or name in fds:
            for a in ("Send", "Sync"):
                if a in t.auto:
                    rows[(f, f"{name}: {a}")] = Row(f, f"{name}: {a}", t.vis, "auto trait")
    return rows


def impl_key(self_name: str) -> str:
    """A Self type as both sides spell it: generics, references and paths dropped."""
    base = re.sub(r"<.*$", "", self_name).replace("&", "").replace("mut ", "").strip()
    return base.split("::")[-1]


def union_inventories(parts: list[tuple[dict, dict, dict]]) -> tuple[dict, dict, dict, list[str]]:
    rows, types, names, conflicts = {}, {}, {}, []
    for r, t, n in parts:
        for k, v in r.items():
            if k in rows and (rows[k].vis, rows[k].kind) != (v.vis, v.kind):
                conflicts.append(f"{k[0]} `{k[1]}`: {rows[k].vis}/{rows[k].kind} vs {v.vis}/{v.kind} across targets")
            rows.setdefault(k, v)
        for k, v in t.items():
            types.setdefault(k, v)
        for f, s in n.items():
            names.setdefault(f, set()).update(s)
    return rows, types, names, conflicts


# ---------------------------------------------------------------------------------------
# the table
# ---------------------------------------------------------------------------------------


def split_cells(line: str) -> list[str]:
    body = line.strip()
    if body.startswith("|"):
        body = body[1:]
    if body.endswith("|"):
        body = body[:-1]
    return [c.strip() for c in body.split(" | ")]


def parse_table(text: str) -> tuple[dict[tuple[str, str], Row], dict[str, tuple[str, str, int]], list[Finding]]:
    rows: dict[tuple[str, str], Row] = {}
    vdeps: dict[str, tuple[str, str, int]] = {}
    errs: list[Finding] = []
    section = None
    for n, line in enumerate(text.splitlines(), 1):
        if line.startswith("## "):
            section = line[3:].strip()
            continue
        if not line.startswith("|") or line.strip() in (HEADER, SEP, VD_HEADER, VD_SEP):
            continue
        cells = split_cells(line)
        if section == "Validation dependencies":
            if len(cells) != 3:
                errs.append(Finding("E11", TABLE, n, f"malformed validation row: {line}"))
                continue
            vdeps[cells[0].strip("`")] = (cells[1], cells[2], n)
            continue
        if section is None or len(cells) != 6:
            errs.append(Finding("E1", TABLE, n, f"malformed row (want 6 cells under a `## <file>` heading): {line}"))
            continue
        item = cells[0].strip("`")
        if (section, item) in rows:
            errs.append(Finding("E1", TABLE, n, f"duplicate row `{item}`"))
        rows[(section, item)] = Row(section, item, cells[1], cells[2], cells[3], cells[4], cells[5], n)
    return rows, vdeps, errs


def render_table(rows: dict[tuple[str, str], Row], vdeps: dict[str, tuple[str, str]], date: str,
                 crates_note: str) -> str:
    out = ["# Perimeter exports — the reviewed table (OWNER_RULINGS §R gate 3)", "",
           f"STATUS: LIVE, {date}. Generated inventory must equal this table (`scripts/ci/perimeter.py exports`).",
           "<!-- perimeter-exports format=2 rustdoc-format=61 -->", "",
           "Every item a perimeter file (`*_unsafe.rs` of a class U crate in kf3's graph) exports to safe code "
           "is a row: the checks it performs on its own inputs, and the test that shows each check "
           "(`label=t:<file>::<test fn>`). Grammar, rules E1-E14 and how a row becomes `OK`: "
           "`docs/design/V3_SEC_PERIMETER.md` §3. The OPEN count is printed by the gate, never stored.",
           "", crates_note, ""]
    for f in sorted({k[0] for k in rows}):
        out += [f"## {f}", "", HEADER, SEP]
        for k in sorted(k for k in rows if k[0] == f):
            r = rows[k]
            out.append("| " + " | ".join(r.cells()) + " |")
        out.append("")
    out += ["## Validation dependencies", "",
            "Same-crate, non-perimeter files that define an item a perimeter file names (E11, derived by "
            "`perimeter.py exports`). VALIDATES: a perimeter memory-safety argument relies on it; the file "
            "is in `scripts/ci/perimeter/sizes.tsv` and gets perimeter review. USES: anything else.", "",
            VD_HEADER, VD_SEP]
    for f in sorted(vdeps):
        role, why = vdeps[f]
        out.append(f"| `{f}` | {role} | {why} |")
    out.append("")
    return "\n".join(out)


def table_counts(text: str) -> tuple[dict[str, int], list[str]]:
    """Rows per file (the sizes.tsv `exports` column) and the VALIDATES files."""
    rows, vdeps, _ = parse_table(text)
    counts: dict[str, int] = {}
    for f, _ in rows:
        counts[f] = counts.get(f, 0) + 1
    return counts, sorted(f for f, (role, _, _) in vdeps.items() if role == "VALIDATES")


# ---------------------------------------------------------------------------------------
# validation dependencies (E11), derived by the tokenizer
# ---------------------------------------------------------------------------------------

DEF_KINDS = ("fn", "struct", "enum", "union", "trait", "type", "const", "static", "macro_rules")


def validation_deps(root: Path, crate_files: dict[str, list[str]], is_perimeter) -> dict[str, set[str]]:
    """file -> the names that link it: every same-crate, non-perimeter file under src/ defining a
    module-level item a perimeter file's code tokens name."""
    out: dict[str, set[str]] = {}
    for crate, files in crate_files.items():
        defs: dict[str, set[str]] = {}
        idents: set[str] = set()
        for f in files:
            s = rslex.Structure(rslex.tokenize_file(root / f, f), f)
            if is_perimeter(f):
                tests = s.test_ranges()
                idents |= {t.text for k, t in enumerate(s.code) if t.kind == "ident" and not rslex.in_ranges(k, tests)}
                continue
            tests = s.test_ranges()
            for it in s.items:
                if it.kind not in DEF_KINDS or not it.name or it.name == "_":
                    continue
                if it.parent is not None and it.parent.kind not in ("mod",):
                    continue
                if rslex.in_ranges(it.kw, tests):
                    continue
                defs.setdefault(it.name, set()).add(f)
        for name in idents & set(defs):
            for f in defs[name]:
                out.setdefault(f, set()).add(name)
    return out


# ---------------------------------------------------------------------------------------
# the gate
# ---------------------------------------------------------------------------------------


def test_fns(root: Path, path: str) -> dict[str, tuple[bool, set[str], int]]:
    """name -> (is a non-ignored #[test], identifiers in its body, line) for fns in `path`."""
    p = root / path
    if not p.exists():
        return {}
    s = rslex.Structure(rslex.tokenize_file(p, path), path)
    out = {}
    for it in s.items:
        if it.kind != "fn" or it.body_open is None:
            continue
        is_test = any(s.attr_is(a, "test") for a in it.attrs)
        ignored = any(s.code[a[0] + 2].is_ident("ignore") for a in it.attrs)
        body = {t.text for t in s.code[it.body_open:it.end + 1] if t.kind == "ident"}
        out[it.name] = (is_test and not ignored, body, s.code[it.kw].line)
    return out


def item_ident(item: str) -> str:
    """The identifier a test body must name: the method or function name."""
    base = item.split("::")[-1]
    base = base.split(">")[-1] if base.startswith("<") else base
    return re.sub(r"\W", "", base)


SHAPE_RULES = ("E4", "E5", "E6", "E7", "E8", "E12", "E14")


def debt_key(rule: str, f: str, item: str) -> str:
    return f"{rule}|{f}|{item}"


def check_rows(root: Path, inv: dict, table: dict, types: dict, carrying: set[str], fds: set[str],
               skip_guards: set[str], base_table: dict | None, e9_baseline: int,
               debt: set[str] | None = None) -> list[Finding]:
    """E1-E14 over the inventory and the table. E3-E3e and E13 bind OK rows (a review's evidence);
    the SHAPE rules (E4-E8, E12, E14: what the owner adopted as gates, "no Copy/Clone on owning
    handles", "private fields on address-carrying handles", a `# Safety` on every unsafe fn) bind
    EVERY row: a violation on an OPEN or LANE row must be listed in `[exports.debt]`, an exact
    per-item baseline that only shrinks. ⊘ They bound OK rows only until 2026-10-04 (review): 437
    of 440 rows were OPEN or LANE, so a new owning handle deriving Copy landed green under the
    skeleton status E1 itself prints."""
    out: list[Finding] = []
    debt = set(debt or ())
    seen_debt: set[str] = set()
    # E1: inventory equals the table
    for k in sorted(inv.keys() - table.keys()):
        r = inv[k]
        out.append(Finding("E1", TABLE, 0, f"add a row under `## {k[0]}`: | `{r.item}` | {r.vis} | "
                                           f"{'<type kind>' if r.kind == 'type' else r.kind} |  |  | OPEN: YYYY-MM-DD: unreviewed |"))
    for k in sorted(table.keys() - inv.keys()):
        out.append(Finding("E1", TABLE, table[k].line, f"remove the row `{k[1]}` from `## {k[0]}`: not exported"))
    tests_cache: dict[str, dict] = {}
    type_by_name = {(t.file, t.name): t for t in types.values()}
    for k in sorted(inv.keys() & table.keys()):
        g, t = inv[k], table[k]
        n = t.line
        # E2: mechanical columns
        if t.vis != g.vis:
            out.append(Finding("E2", TABLE, n, f"`{t.item}` vis {t.vis!r} != generated {g.vis!r}"))
        if g.kind == "type":
            if t.kind not in TYPE_KINDS:
                out.append(Finding("E2", TABLE, n, f"`{t.item}` is a type: kind must be one of {TYPE_KINDS}"))
        elif t.kind != g.kind:
            out.append(Finding("E2", TABLE, n, f"`{t.item}` kind {t.kind!r} != generated {g.kind!r}"))
        # E10: every OPEN/LANE row carries a dated reason (stricter than "new or changed rows": the
        # grammar holds for every row, so no row can lose its date later either)
        if not STATUS_RE.match(t.status):
            out.append(Finding("E10", TABLE, n, f"`{t.item}` status must be OK, `OPEN: YYYY-MM-DD: …` or "
                                                f"`LANE:<branch>: YYYY-MM-DD: …`, got {t.status!r}"))
        elif base_table is not None and k in base_table and base_table[k].status == "OK" and t.status != "OK":
            # an OK row going back to OPEN/LANE is a regression the reviewer must see named
            print(f"  E10 note: `{t.item}` was OK at the base and is now {t.status!r}")
        shape = shape_rules(g, t, n, type_by_name, carrying, fds)
        if t.status == "OK":
            out += ok_row_rules(root, g, t, n, skip_guards, tests_cache)
            out += shape
            continue
        for f in shape:
            key = debt_key(f.rule, k[0], k[1])
            if key in debt:
                seen_debt.add(key)
            else:
                out.append(Finding(f.rule, TABLE, n, f"{f.msg} (an {t.status.split(':')[0]} row: the shape rules bind "
                                                     f"every row; fix it, or list `{key}` in perimeter.toml "
                                                     "[exports.debt])"))
    for key in sorted(k for k in debt - seen_debt if k.split("|")[0] in SHAPE_RULES):
        out.append(Finding("E15", TABLE, 0, f"[exports.debt] lists `{key}`, which no longer violates: remove it "
                                            "(the debt only shrinks, exactly)"))
    # E9: safe extern fns taking a raw pointer, an exact baseline
    e9 = sorted(k for k, r in inv.items() if r.kind == "safe extern fn" and type_contains(
        [x[1] for x in r.meta.get("fn", {}).get("inner", {}).get("function", {}).get("sig", {}).get("inputs", [])],
        lambda x: isinstance(x, dict) and "raw_pointer" in x))
    if len(e9) != e9_baseline:
        out.append(Finding("E9", TABLE, 0, f"{len(e9)} safe `extern \"C\"` fns take a raw pointer; the baseline is "
                                           f"{e9_baseline} and moves only DOWN, exactly: "
                                           + ", ".join(f"{f}::{i}" for f, i in e9)))
    return out


def ok_row_rules(root: Path, g: Row, t: Row, n: int, skip_guards: set[str], cache: dict) -> list[Finding]:
    """E3-E3e and E13: an OK row's evidence."""
    out = []
    labels = [x.strip() for x in t.checks.split(";") if x.strip()]
    refs: dict[str, list[str]] = {}
    for pair in [x.strip() for x in t.tests.split(";") if x.strip()]:
        if "=" not in pair:
            out.append(Finding("E3", TABLE, n, f"`{t.item}`: test entry {pair!r} is not `label=ref`"))
            continue
        lab, ref = pair.split("=", 1)
        refs.setdefault(lab.strip(), []).append(ref.strip())
    needs_checks = t.kind in ("safe fn", "safe extern fn", "trait impl") or g.meta.get("trait_def")
    if needs_checks and not labels:
        out.append(Finding("E3", TABLE, n, f"`{t.item}` is OK with an empty checks cell"))
    for lab in labels:
        good = False
        for ref in refs.get(lab, []):
            m = REF_RE.match(ref)
            if not m:
                out.append(Finding("E3", TABLE, n, f"`{t.item}`: ref {ref!r} is not t:/ui:/box:/equiv:"))
                continue
            kind, target = m.groups()
            if kind == "t":
                path, _, fn = target.rpartition("::")
                tf = cache.setdefault(path, test_fns(root, path))
                if fn not in tf:
                    out.append(Finding("E3", TABLE, n, f"`{t.item}`: {lab}: no fn `{fn}` in {path}"))
                    continue
                is_test, body, _ = tf[fn]
                if not is_test:
                    out.append(Finding("E3", TABLE, n, f"`{t.item}`: {lab}: {path}::{fn} is not a running #[test]"))
                    continue
                if item_ident(t.item) not in body:
                    out.append(Finding("E3b", TABLE, n, f"`{t.item}`: {lab}: {path}::{fn} never names "
                                                        f"`{item_ident(t.item)}`"))
                    continue
                if body & skip_guards:
                    out.append(Finding("E3d", TABLE, n, f"`{t.item}`: {lab}: {path}::{fn} is skip-guarded "
                                                        f"({sorted(body & skip_guards)}); it does not run in CI"))
                    continue
                good = True
            elif kind == "ui":
                path, _, code = target.partition("#")
                stderr = root / Path(path).with_suffix(".stderr")
                if not (root / path).exists() or not stderr.exists() or code not in stderr.read_text():
                    out.append(Finding("E3e", TABLE, n, f"`{t.item}`: {lab}: {path}'s .stderr lacks {code!r}"))
                    continue
                good = True
            elif kind == "equiv":
                good = good or False
        if not good:
            out.append(Finding("E3", TABLE, n, f"`{t.item}`: check `{lab}` has no t:/ui: test that runs in CI"))
    # E13: safe traits
    if t.kind == "trait" and not ({"sealed", "no-reliance"} & set(labels)):
        out.append(Finding("E13", TABLE, n, f"safe trait `{t.item}`: state `sealed` or `no-reliance` in checks"))
    return out


def shape_rules(g: Row, t: Row, n: int, type_by_name: dict, carrying: set[str], fds: set[str]) -> list[Finding]:
    """E4-E8, E12, E14: properties of the item itself, whatever the row's status."""
    out = []
    labels = [x.strip() for x in t.checks.split(";") if x.strip()]
    # E4: # Safety
    if t.kind in ("unsafe fn", "unsafe extern fn", "unsafe trait"):
        docs = (g.meta.get("fn") or g.meta.get("trait") or {}).get("docs") or ""
        if not re.search(r"(?m)^#+\s*Safety\b", docs):
            out.append(Finding("E4", TABLE, n, f"`{t.item}` is unsafe and has no `# Safety` heading"))
    # E5-E8: types
    ti = type_by_name.get((t.file, t.item))
    if ti is not None:
        nonpriv = [fname for fname, vis, _ in ti.fields if vis_text(vis) not in ("default",) and not (
            isinstance(vis, dict) and "restricted" in vis and vis["restricted"].get("path", "").endswith(
                PurePosixPath(t.file).stem))]
        owning_reason = ("Drop" in ti.traits or ti.name in fds and any(
            type_contains(fty, lambda x: isinstance(x, dict) and "resolved_path" in x and
                          x["resolved_path"]["path"].split("::")[-1] == "OwnedFd") for _, _, fty in ti.fields))
        if t.kind == "owning handle":
            if {"Copy", "Clone"} & ti.traits:
                out.append(Finding("E5", TABLE, n, f"owning handle `{t.item}` implements {sorted({'Copy', 'Clone'} & ti.traits)}"))
            if nonpriv:
                out.append(Finding("E5", TABLE, n, f"owning handle `{t.item}` has non-private fields {nonpriv}"))
        elif owning_reason:
            out.append(Finding("E5", TABLE, n, f"`{t.item}` has Drop or owns an fd: classify it `owning handle`"))
        if ti.name in carrying:
            if t.kind == "plain data":
                out.append(Finding("E6", TABLE, n, f"`{t.item}` carries an address: never `plain data`"))
            if nonpriv and t.kind != "FFI struct":
                out.append(Finding("E6", TABLE, n, f"address-carrying `{t.item}` has non-private fields {nonpriv}"))
            if t.kind == "FFI struct" and not ti.repr_c:
                out.append(Finding("E6", TABLE, n, f"`{t.item}` is `FFI struct` without repr(C)"))
        if ti.name in fds and t.kind == "plain data":
            out.append(Finding("E7", TABLE, n, f"`{t.item}` carries a descriptor: never `plain data`"))
        if t.kind == "borrowed view" and not ti.has_lifetime:
            out.append(Finding("E8", TABLE, n, f"borrowed view `{t.item}` has no lifetime parameter"))
    # E12: generic exports
    fn = g.meta.get("fn")
    if fn is not None and t.kind in ("safe fn", "safe extern fn"):
        fi = fn["inner"]["function"]
        generic = any("type" in p.get("kind", {}) for p in fi["generics"]["params"]) or type_contains(
            fi["sig"]["inputs"], lambda x: isinstance(x, dict) and ("impl_trait" in x or "dyn_trait" in x
                                                                    or "function_pointer" in x))
        if generic:
            for lab in ("untrusted-impls", "panic-safe"):
                if lab not in labels:
                    out.append(Finding("E12", TABLE, n, f"generic export `{t.item}` needs the check `{lab}` with a test"))
    # E14: statics of an address-carrying type
    st = g.meta.get("static")
    if st is not None and (type_contains(st.get("type"), lambda x: isinstance(x, dict) and "raw_pointer" in x)
                           or render_type(st.get("type")).split("<")[0] in carrying):
        out.append(Finding("E14", TABLE, n, f"static `{t.item}` has an address-carrying type; it cannot be OK"))
    return out


def has_safety_heading(text: str) -> bool:
    return re.search(r"(?m)^\s*#+\s*Safety\b", text) is not None


def item_docs(s: rslex.Structure, it: rslex.Item) -> str:
    """The doc text written on an item: its outer doc comments (between its attributes too) and
    its `#[doc = "…"]` string literals."""
    c = s.code
    pos = {id(t): i for i, t in enumerate(s.all)}
    lo = pos[id(c[it.start])]
    j = lo - 1
    while j >= 0 and not s.all[j].is_code:
        j -= 1
    text = []
    for t in s.all[j + 1:pos[id(c[it.kw])]]:
        if t.kind == "doc" and not t.inner:
            text.append(t.text[3:] if t.text.startswith("///") else t.text[3:-2])
    for a in it.attrs:
        toks = c[a[0]:a[1] + 1]
        for k, t in enumerate(toks):
            if t.is_ident("doc") and k + 2 < len(toks) and toks[k + 1].is_punct("=") and toks[k + 2].kind == "str":
                text.append(toks[k + 2].text)
    return "\n".join(text)


def unsafe_fn_safety_findings(root: Path, files: list[str], debt: set[str]) -> list[Finding]:
    """E4, by the tokenizer, for EVERY `unsafe fn` and `unsafe trait` in a perimeter file, private
    ones included (they have no rows, and rule (a) makes them the place a precondition lives):
    each has a `# Safety` heading. Exact baseline `E4L|<file>|<Owner::>fn` in [exports.debt]."""
    out, seen = [], set()
    for f in files:
        s = rslex.Structure(rslex.tokenize_file(root / f, f), f)
        tests = s.test_ranges()
        c = s.code
        for it in s.items:
            if it.kind not in ("fn", "trait") or rslex.KEYWORD_UNSAFE not in it.qualifiers:
                continue
            if rslex.in_ranges(it.kw, tests) or (it.parent is not None and it.parent.kind == "extern_block"):
                continue
            if it.kind == "fn" and it.parent is not None and it.parent.kind == "impl" and " as " in it.parent.name:
                continue  # an impl of a trait method: its contract is the trait's `# Safety`
            if has_safety_heading(item_docs(s, it)):
                continue
            owner = f"{it.parent.name}::" if it.parent is not None and it.parent.kind in ("impl", "trait") else ""
            key = debt_key("E4L", f, owner + it.name)
            if key in debt:
                seen.add(key)
                continue
            out.append(Finding("E4L", f, c[it.kw].line, f"`{owner}{it.name}` is unsafe and has no `# Safety` heading "
                                                        f"(every unsafe fn in a perimeter file, private or not; or list "
                                                        f"`{key}` in [exports.debt])"))
    for key in sorted(k for k in debt - seen if k.startswith("E4L|") and k.split("|")[1] in files):
        out.append(Finding("E15", TABLE, 0, f"[exports.debt] lists `{key}`, which no longer violates: remove it"))
    return out


def cfg_predicates(toks: list[rslex.Tok]) -> list[list[rslex.Tok]]:
    """Every `cfg(P)` and `cfg_attr(P, …)` predicate inside one attribute's tokens, at ANY depth:
    `cfg_attr(all(), cfg_attr(not(doc), derive(Clone)))` has two. ⊘ Reading only the outer one
    let `not(doc)` through, and rustdoc (which sets `cfg(doc)`) then never saw the Clone impl
    (review 2026-10-04)."""
    out = []
    for j, t in enumerate(toks):
        if not (t.is_ident("cfg") or t.is_ident("cfg_attr")) or j + 1 >= len(toks) or not toks[j + 1].is_punct("("):
            continue
        depth, pred = 0, []
        for x in toks[j + 2:]:
            if x.is_punct("(") or x.is_punct("["):
                depth += 1
            elif x.is_punct(")") or x.is_punct("]"):
                if depth == 0:
                    break
                depth -= 1
            if depth == 0 and x.is_punct(",") and t.is_ident("cfg_attr"):
                break
            pred.append(x)
        out.append(pred)
    return out


def cfg_atoms(pred: list[rslex.Tok]) -> list[str]:
    atoms, k = [], 0
    while k < len(pred):
        x = pred[k]
        if x.kind == "ident" and x.text in ("any", "all", "not") and k + 1 < len(pred) and pred[k + 1].is_punct("("):
            k += 2
            continue
        if x.kind == "ident":
            if k + 2 < len(pred) and pred[k + 1].is_punct("=") and pred[k + 2].kind == "str":
                atoms.append(f"{x.text} = {pred[k + 2].text}")
                k += 3
                continue
            atoms.append(x.text)
        k += 1
    return atoms


def cfg_findings(root: Path, files: list[str], allowed: set[str], features: dict[str, set[str]],
                 crate_of) -> list[Finding]:
    """E1c: cfg/cfg_attr predicates on items in perimeter files, nested ones included, use only
    the allowed atoms and the crate's own features; `debug_assertions` is banned there, and so is
    `doc` (rustdoc sets it: an item under `not(doc)` is invisible to the generator)."""
    out = []
    for f in files:
        s = rslex.Structure(rslex.tokenize_file(root / f, f), f)
        c = s.code
        feats = features.get(crate_of(f), set())
        for o, e, _ in s.attrs:
            for pred in cfg_predicates(c[o:e + 1]):
                for a in cfg_atoms(pred):
                    ok = a in allowed or (a.startswith("feature = ") and a.split(" = ", 1)[1].strip('"') in feats)
                    if a in ("debug_assertions", "doc") or not ok:
                        out.append(Finding("E1c", f, c[o].line, f"cfg predicate `{a}` on a perimeter item: only "
                                                                f"{sorted(allowed)} and the crate's features"))
    uniq = {(x.rule, x.path, x.line, x.msg): x for x in out}
    return [uniq[k] for k in sorted(uniq)]


def lexer_impl_pairs(s: rslex.Structure) -> list[tuple[str, str, int]]:
    """(Self, Trait, line) for every trait impl and every derived trait the tokenizer sees in a
    file, at ANY depth (fn and const bodies, inline modules), outside `#[cfg(test)]` and outside
    macro bodies (whose `$t` it cannot resolve). A derive is found in any attribute, cfg_attr
    included, on a struct, enum or union."""
    c = s.code
    tests = s.test_ranges()
    macros = [(it.body_open, it.end) for it in s.items if it.kind == "macro_rules" and it.body_open is not None]
    out = []
    for it in s.items:
        if rslex.in_ranges(it.kw, tests) or rslex.in_ranges(it.kw, macros):
            continue
        if it.kind == "impl" and it.name.startswith("<") and " as " in it.name:
            ty, _, tr = it.name[1:-1].partition(" as ")
            out.append((impl_key(ty), re.sub(r"<.*$", "", tr).split("::")[-1], c[it.kw].line))
        elif it.kind in ("struct", "enum", "union"):
            for a in it.attrs:
                toks = c[a[0]:a[1] + 1]
                for j, t in enumerate(toks):
                    if t.is_ident("derive") and j + 1 < len(toks) and toks[j + 1].is_punct("("):
                        depth, last = 0, None
                        for x in toks[j + 2:]:
                            if x.is_punct("(") or x.is_punct("<"):
                                depth += 1
                            elif x.is_punct(")") or x.is_punct(">"):
                                if depth == 0:
                                    break
                                depth -= 1
                            elif x.is_punct(",") and depth == 0:
                                if last:
                                    out.append((it.name, last, c[it.kw].line))
                                last = None
                            elif x.kind == "ident" and depth == 0:
                                last = x.text  # a path `core::clone::Clone` names its last segment
                        if last:
                            out.append((it.name, last, c[it.kw].line))
    return out


def reach_findings(root: Path, files: list[str], names: dict[str, set[str]]) -> list[Finding]:
    """E1b: every perimeter file has an item in the rustdoc union, and every item the tokenizer sees
    at item level (outside cfg(test), not in a fn body) is in it. Non-exported `macro_rules!` are
    out: rustdoc documents only exported macros."""
    out = []
    for f in files:
        for what in sorted(names.get(("spliced", f), ())):
            out.append(Finding("E1d", f, 0, f"`{what}`: code of this perimeter file compiled into another "
                                             "file's module (include!, or an alias of it)"))
        got = names.get(f, set())
        if not got:
            out.append(Finding("E1b", f, 0, "no item of this perimeter file reached the rustdoc union "
                                            "(unreached, or every item hidden from the generator)"))
            continue
        s = rslex.Structure(rslex.tokenize_file(root / f, f), f)
        impls = names.get(("impls", f), set())
        for ty, tr, ln in lexer_impl_pairs(s):
            if (ty, tr) not in impls:
                out.append(Finding("E1b", f, ln, f"`impl {tr} for {ty}` (written or derived) is missing from the "
                                                 "rustdoc union: an impl hidden from the generator (cfg(doc)) is a "
                                                 "row nobody reviews"))
        tests = s.test_ranges()
        for it in s.items:
            if it.kind not in ("fn", "struct", "enum", "union", "trait", "type", "const", "static"):
                continue
            if not it.name or it.name == "_" or rslex.in_ranges(it.kw, tests):
                continue
            p = it.parent
            if p is not None and p.kind not in ("mod", "impl", "trait"):
                continue
            if p is not None and p.kind == "impl" and p.parent is not None and p.parent.kind not in ("mod", "const"):
                continue
            if it.name not in got:
                out.append(Finding("E1b", f, s.code[it.kw].line, f"`{it.name}` ({it.kind}) is missing from the "
                                                                 "rustdoc union (cfg gap, doc(hidden), or a generator bug)"))
    return out


# ---------------------------------------------------------------------------------------
# K4: kf3.h's prototypes against the Rust signatures (V3_SEC_PERIMETER.md §8.1)
# ---------------------------------------------------------------------------------------

C_PRIMS = {"u8": "uint8_t", "u16": "uint16_t", "u32": "uint32_t", "u64": "uint64_t", "i8": "int8_t",
           "i16": "int16_t", "i32": "int32_t", "i64": "int64_t", "usize": "size_t", "isize": "ptrdiff_t",
           "c_void": "void", "c_char": "char", "c_int": "int", "c_uint": "unsigned int", "c_long": "long",
           "c_ulong": "unsigned long", "bool": "_Bool"}


def c_type(doc: Doc, t, decl: str = "") -> str:
    """A C declarator for a Rust FFI type (a fixed map; anything else refuses by name)."""
    k, v = next(iter(t.items()))
    if k == "primitive":
        if v not in C_PRIMS:
            raise ValueError(f"no C type for primitive {v}")
        return f"{C_PRIMS[v]}{(' ' + decl) if decl else ''}"
    if k == "raw_pointer":
        inner = v["type"]
        if "raw_pointer" in inner:  # `*mut *const T` is `const T **`; `*const *mut T` is `T *const *`
            return c_type(doc, inner, ("*" if v.get("is_mutable") else "const *") + decl)
        return f"{'' if v.get('is_mutable') else 'const '}{c_type(doc, inner)} *{decl}"
    if k == "resolved_path":
        name = v["path"].split("::")[-1]
        if name in C_PRIMS:
            return f"{C_PRIMS[name]}{(' ' + decl) if decl else ''}"
        if name == "Option" and v.get("args"):
            return c_type(doc, v["args"]["angle_bracketed"]["args"][0]["type"], decl)
        target = doc.idx.get(str(v.get("id")))
        if target is not None and Doc.kind(target) == "type_alias":
            return c_type(doc, target["inner"]["type_alias"]["type"], decl)
        return f"{name}{(' ' + decl) if decl else ''}"
    if k == "function_pointer":
        sig = v["sig"]
        params = ", ".join(c_type(doc, x[1]) for x in sig["inputs"]) or "void"
        ret = c_type(doc, sig["output"]) if sig.get("output") else "void"
        return f"{ret} (*{decl})({params})"
    raise ValueError(f"no C type for {k}")


def k4_prototypes(doc: Doc) -> list[tuple[str, str]]:
    out = []
    for it in doc.idx.values():
        if Doc.kind(it) != "function" or "no_mangle" not in it.get("attrs", []):
            continue
        fi = it["inner"]["function"]
        params = ", ".join(c_type(doc, ty, name) for name, ty in fi["sig"]["inputs"]) or "void"
        ret = c_type(doc, fi["sig"]["output"]) if fi["sig"].get("output") else "void"
        out.append((it["name"], f"{ret} {it['name']}({params});"))
    return sorted(out)


def k4_compile(root: Path, protos: list[str], cc: str = "cc") -> tuple[int, str]:
    src = f'#include "{root}/qemu/hw/misc/kf3/kf3.h"\n' + "\n".join(protos) + "\n"
    r = subprocess.run([cc, "-std=c11", "-fsyntax-only", "-Werror", "-Wall", "-x", "c", "-"], input=src,
                       capture_output=True, text=True)
    return r.returncode, r.stderr


# ---------------------------------------------------------------------------------------
# E3c: mutation evidence for OK rows (nightly, perimeter-mutants.yml)
# ---------------------------------------------------------------------------------------


def norm_item(item: str) -> str:
    """`<X as From<RawError>>::from` -> `<X as From>::from`: generic arguments dropped."""
    prev = None
    while prev != item:
        prev, item = item, re.sub(r"(\w)<[^<>]*>", r"\1", item)
    return item


def fn_row_name(s: rslex.Structure, line: int, name: str) -> str | None:
    """The table identity of the fn `name` declared on `line`."""
    for it in s.items:
        if it.kind == "fn" and it.name == name and s.code[it.start].line <= line <= s.code[it.end].line:
            p = it.parent
            if p is not None and p.kind == "impl":
                return f"{p.name}::{name}"
            if p is not None and p.kind == "trait":
                return f"{p.name}::{name}"
            return name
    return None


FN_ROW_KINDS = ("safe fn", "unsafe fn", "safe extern fn", "unsafe extern fn")


def call_graph(s: rslex.Structure) -> dict[str, set[str]]:
    """fn name -> the names it calls, by the tokenizer: an identifier followed by `(` or `::<`,
    or after `.`. Same-named fns merge, which over-charges (the safe direction)."""
    c = s.code
    out: dict[str, set[str]] = {}
    for it in s.items:
        if it.kind != "fn" or it.body_open is None:
            continue
        called = out.setdefault(it.name, set())
        for k in range(it.body_open + 1, it.end):
            t = c[k]
            if t.kind == "ident" and ((k + 1 < len(c) and (c[k + 1].is_punct("(") or c[k + 1].is_punct(":")))
                                      or (k > 0 and c[k - 1].is_punct("."))):
                called.add(t.text)
    return out


def closure_of(graph: dict[str, set[str]], start: str) -> set[str]:
    seen, stack = set(), [start]
    while stack:
        x = stack.pop()
        if x in seen:
            continue
        seen.add(x)
        stack.extend(graph.get(x, ()))
    return seen


def mutant_findings(root: Path, outcomes: list[dict], table: dict, validates: list[str] = (),
                    require_caught_under: str | None = None) -> tuple[list[Finding], dict]:
    """E3c: an OK fn row has ZERO missed mutants, apart from its `equiv:` entries, counting the
    mutants of the code its check actually lives in: its own body, every same-file fn it calls
    (transitively), and every VALIDATES-file fn those bodies name. With `require_caught_under`
    (a full run over that crate), each OK fn row there also needs at least one CAUGHT mutant or
    an `equiv:` entry, and the unmutated baseline must have succeeded.

    ⊘ Until 2026-10-04 (review) only the row's own body counted: `KvmVm::adopt` was OK on "1
    unviable" (zero caught) while its check lived in the private `confirm_is_a_vm`, a VALIDATES
    file was never mutated, and a run whose baseline failed (so no MissedMutant at all) passed."""
    ok_rows = {(f, norm_item(r.item)): r for (f, _), r in table.items() if r.status == "OK" and r.kind in FN_ROW_KINDS}
    structs: dict[str, rslex.Structure] = {}

    def struct(f: str) -> rslex.Structure | None:
        if f not in structs:
            p = root / f
            structs[f] = rslex.Structure(rslex.tokenize_file(p, f), f) if p.exists() else None
        return structs[f]

    # for each OK row: the fn names whose mutants are charged to it, per file
    charged: dict[tuple[str, str], dict[str, set[str]]] = {}
    for (f, item) in ok_rows:
        s = struct(f)
        if s is None:
            continue
        own = item.split("::")[-1]
        names = closure_of(call_graph(s), own)
        charged[(f, item)] = {f: names}
        anchored = "/" + f
        crate_src = anchored[1:anchored.rfind("/src/") + 4]  # crates/<x>/src (or a fixture's src)
        bodies = [s.code[it.body_open:it.end] for it in s.items
                  if it.kind == "fn" and it.name in names and it.body_open is not None]
        idents = {t.text for b in bodies for t in b if t.kind == "ident"}
        local_fns = {it.name for it in s.items if it.kind == "fn"}
        for v in validates:
            if v.startswith(crate_src + "/"):
                vs = struct(v)
                if vs is None:
                    continue
                vg = call_graph(vs)
                stem = PurePosixPath(v).stem
                # A name the perimeter file ALSO defines counts only when written `stem::name`.
                used = {n for n in idents & set(vg) if n not in local_fns or any(
                    b[k].is_ident(stem) and b[k + 1].is_punct(":") and b[k + 2].is_punct(":") and b[k + 3].is_ident(n)
                    for b in bodies for k in range(len(b) - 3))}
                hit = set().union(*(closure_of(vg, n) for n in used)) if used else set()
                if hit:
                    charged[(f, item)][v] = hit
    missed: dict[tuple[str, str], list[str]] = {}
    caught: dict[tuple[str, str], int] = {}
    baseline = None
    for o in outcomes:
        sc = o.get("scenario")
        if sc == "Baseline" or (isinstance(sc, dict) and "Baseline" in sc):
            baseline = o.get("summary")
            continue
        m = sc.get("Mutant") if isinstance(sc, dict) else None
        if not m:
            continue
        f = m["file"]
        fn = (m.get("function") or {}).get("function_name")
        if not fn:
            continue
        s = struct(f)
        if s is None:
            continue
        # the mutant's own line: the function span starts at its doc comment, not its `fn`
        name = fn_row_name(s, m["span"]["start"]["line"], fn.split("::")[-1])
        if name is None:
            continue
        bare = name.split("::")[-1]
        desc = m["name"].split(": ", 1)[-1]
        targets = {key for key, per in charged.items() if bare in per.get(f, ())}
        if o.get("summary") == "MissedMutant":
            missed.setdefault((f, norm_item(name)), []).append(desc)
            for key in targets:
                missed.setdefault(("charged",) + key, []).append(f"{desc} (in `{name}`, {f})")
        elif o.get("summary") == "CaughtMutant":
            for key in targets:
                caught[key] = caught.get(key, 0) + 1
    out = []
    for key, r in sorted(ok_rows.items()):
        descs = missed.get(("charged",) + key, [])
        equiv = {x.split("=", 1)[1][len("equiv:"):].strip() for x in r.tests.split(";")
                 if "=" in x and x.split("=", 1)[1].strip().startswith("equiv:")}
        left = [d for d in descs if d.split(" (in `")[0] not in equiv]
        if left:
            out.append(Finding("E3c", TABLE, r.line, f"OK row `{r.item}` has {len(left)} MISSED mutant(s) in the code "
                                                     "its checks live in: " + "; ".join(left[:6])))
        if require_caught_under is not None and key[0].startswith(require_caught_under.rstrip("/") + "/") \
                and caught.get(key, 0) == 0 and not equiv:
            out.append(Finding("E3c", TABLE, r.line, f"OK row `{r.item}` has ZERO caught mutants: no mutation shows "
                                                     "its test checks anything (add one, or an `equiv:` entry)"))
    if require_caught_under is not None and baseline != "Success":
        out.append(Finding("E3c", TABLE, 0, f"the unmutated baseline is {baseline!r}, not 'Success': a run whose "
                                            "tests never passed has no missed mutant to report"))
    return out, {k: v for k, v in missed.items() if k[0] != "charged"}
