"""Validate rex schema protos and ExtractionRecipe YAML files against the contracts in proto/.

Usage (from the repo root):
    uv run --with protobuf --with pyyaml python scripts/validate_recipes.py recipes/**/*.yaml

Checks:
  schemas  - every rex.schema.* message has no map<> fields (decisions.md U1/U3)
  recipes  - parse strictly into rex.extract.v1.ExtractionRecipe (unknown keys = error);
             `schema` exists; every rule path resolves in the schema (nested rules relative to the item);
             `each` only on repeated message fields; nested `fields` only with `each`; paths unique per level;
             `field` references resolve; `extends` names a loaded recipe; regexes compile and stay in the
             RE2 subset (no lookaround, no backreferences).
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml
from google.protobuf import descriptor_pb2, descriptor_pool, json_format, message_factory
from google.protobuf.descriptor import FieldDescriptor

ROOT = Path(__file__).resolve().parent.parent
PROTO_DIR = ROOT / "proto"
RE2_FORBIDDEN = re.compile(r"\(\?<?[=!]|\\[1-9]")


def build_pool() -> descriptor_pool.DescriptorPool:
    protos = sorted(str(p.relative_to(PROTO_DIR)) for p in PROTO_DIR.rglob("*.proto"))
    with tempfile.NamedTemporaryFile(suffix=".pb") as out:
        subprocess.run(
            ["protoc", f"-I{PROTO_DIR}", "-I/usr/include", "--include_imports", f"--descriptor_set_out={out.name}", *protos],
            check=True,
        )
        fds = descriptor_pb2.FileDescriptorSet.FromString(Path(out.name).read_bytes())
    pool = descriptor_pool.DescriptorPool()
    for f in fds.file:
        pool.Add(f)
    for f in fds.file:
        pool.FindFileByName(f.name)
    return pool, [f.name for f in fds.file]


def check_schemas(pool, files) -> list[str]:
    errors = []
    for name in files:
        fd = pool.FindFileByName(name)
        if not fd.package.startswith("rex.schema.") or fd.package == "rex.schema.v1":
            continue
        for msg in fd.message_types_by_name.values():
            for field in msg.fields:
                if field.message_type and field.message_type.GetOptions().map_entry:
                    errors.append(f"schema {msg.full_name}.{field.name}: map<> fields are not allowed")
    return errors


def check_regex(where: str, pattern: str, errors: list[str]) -> None:
    if RE2_FORBIDDEN.search(pattern):
        errors.append(f"{where}: regex {pattern!r} is outside the RE2 subset (lookaround/backreference)")
        return
    try:
        re.compile(pattern)
    except re.error as e:
        errors.append(f"{where}: regex {pattern!r} does not compile: {e}")


def walk_regexes(where: str, obj, errors: list[str]) -> None:
    if isinstance(obj, dict):
        for k, v in obj.items():
            if k in ("include", "exclude") and isinstance(v, list):
                for p in v:
                    check_regex(f"{where}.{k}", p, errors)
            elif k in ("regex", "regex_replace", "split", "filter") and isinstance(v, dict) and "pattern" in v:
                check_regex(f"{where}.{k}", v["pattern"], errors)
            else:
                walk_regexes(f"{where}.{k}", v, errors)
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            walk_regexes(f"{where}[{i}]", v, errors)


def resolve(msg_desc, path: str):
    """Resolve a dotted path relative to msg_desc; returns the final FieldDescriptor or None."""
    desc, field = msg_desc, None
    for part in path.split("."):
        if desc is None:
            return None
        field = desc.fields_by_name.get(part)
        if field is None:
            return None
        desc = field.message_type
    return field


def check_rules(where, msg_desc, rules, errors, all_paths, prefix=""):
    seen = set()
    for i, rule in enumerate(rules):
        path = rule.get("path", "")
        loc = f"{where}.fields[{i}]({prefix}{path})"
        if path in seen:
            errors.append(f"{loc}: duplicate path at this level")
        seen.add(path)
        field = resolve(msg_desc, path)
        if field is None:
            errors.append(f"{loc}: path not found in {msg_desc.full_name}")
            continue
        all_paths.add(prefix + path)
        repeated = field.is_repeated if hasattr(field, "is_repeated") else field.label == FieldDescriptor.LABEL_REPEATED
        is_repeated_msg = repeated and field.message_type is not None
        if rule.get("each") and not is_repeated_msg:
            errors.append(f"{loc}: `each` is only valid on repeated message fields")
        if rule.get("fields"):
            if not rule.get("each"):
                errors.append(f"{loc}: nested `fields` require `each`")
            else:
                check_rules(loc, field.message_type, rule["fields"], errors, all_paths, prefix + path + ".")


def check_field_refs(where, rules, all_paths, errors, prefix=""):
    for rule in rules:
        for c in rule.get("candidates", []):
            ref = c.get("field", {}).get("path") if isinstance(c.get("field"), dict) else None
            if ref is not None and ref not in all_paths and (prefix + ref) not in all_paths:
                errors.append(f"{where}({prefix}{rule.get('path')}): field reference {ref!r} is not a rule path")
        if rule.get("fields"):
            check_field_refs(where, rule["fields"], all_paths, errors, prefix + rule["path"] + ".")


def main(argv: list[str]) -> int:
    pool, files = build_pool()
    errors = check_schemas(pool, files)
    recipe_cls = message_factory.GetMessageClass(pool.FindMessageTypeByName("rex.extract.v1.ExtractionRecipe"))
    docs = {}
    for name in argv:
        data = yaml.safe_load(Path(name).read_text())
        try:
            json_format.ParseDict(data, recipe_cls(), descriptor_pool=pool)
        except json_format.ParseError as e:
            errors.append(f"{name}: does not parse as ExtractionRecipe: {e}")
            continue
        docs[data["id"]] = (name, data)
    for rid, (name, data) in docs.items():
        before = len(errors)
        try:
            schema = pool.FindMessageTypeByName(data["schema"])
        except KeyError:
            errors.append(f"{name}: unknown schema {data['schema']!r}")
            continue
        if data.get("extends") and data["extends"] not in docs:
            errors.append(f"{name}: extends {data['extends']!r}, which is not among the validated recipes")
        all_paths: set[str] = set()
        check_rules(name, schema, data.get("fields", []), errors, all_paths)
        if data.get("extends") in docs:
            base = docs[data["extends"]][1]
            check_rules(name + "<base>", schema, base.get("fields", []), [], all_paths)
        check_field_refs(name, data.get("fields", []), all_paths, errors)
        walk_regexes(name, data, errors)
        if len(errors) == before:
            print(f"ok  {name}  ({rid} -> {schema.full_name}, {len(data.get('fields', []))} rules)")
    for e in errors:
        print(f"ERR {e}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
