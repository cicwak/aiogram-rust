#!/usr/bin/env python3
from __future__ import annotations

import ast
import json
import re
import subprocess
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
UPSTREAM = ROOT / "aiogram"


def assignment(path: Path, name: str) -> str:
    module = ast.parse(path.read_text())
    for node in module.body:
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name) and target.id == name:
                    value = ast.literal_eval(node.value)
                    if isinstance(value, str):
                        return value
    raise RuntimeError(f"{name} not found in {path}")


def generated_count(path: Path, constant: str) -> int:
    match = re.search(
        rf"pub const {re.escape(constant)}: usize = (\d+);",
        path.read_text(),
    )
    if not match:
        raise RuntimeError(f"{constant} not found in {path}")
    return int(match.group(1))


def require(actual: object, expected: object, label: str) -> None:
    if actual != expected:
        raise RuntimeError(f"{label}: expected {expected!r}, got {actual!r}")


def main() -> None:
    compatibility = tomllib.loads((ROOT / "compatibility.toml").read_text())
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    require(
        cargo["package"]["version"],
        compatibility["port"]["version"],
        "Rust port version",
    )
    require(
        cargo["package"]["metadata"]["aiogram-compat"]["aiogram-version"],
        compatibility["upstream"]["aiogram"]["version"],
        "Cargo aiogram metadata",
    )
    require(
        cargo["package"]["metadata"]["aiogram-compat"]["aiogram-commit"],
        compatibility["upstream"]["aiogram"]["commit"],
        "Cargo aiogram commit metadata",
    )
    require(
        cargo["package"]["metadata"]["aiogram-compat"]["telegram-bot-api-version"],
        compatibility["upstream"]["telegram_bot_api"]["version"],
        "Cargo Bot API metadata",
    )

    commit = subprocess.check_output(
        ["git", "-C", str(UPSTREAM), "rev-parse", "HEAD"], text=True
    ).strip()
    require(commit, compatibility["upstream"]["aiogram"]["commit"], "aiogram commit")
    require(
        assignment(UPSTREAM / "aiogram" / "__meta__.py", "__version__"),
        compatibility["upstream"]["aiogram"]["version"],
        "aiogram version",
    )
    require(
        assignment(UPSTREAM / "aiogram" / "__meta__.py", "__api_version__"),
        compatibility["upstream"]["telegram_bot_api"]["version"],
        "aiogram Bot API version",
    )
    schema = json.loads((UPSTREAM / ".butcher" / "schema" / "schema.json").read_text())
    require(
        schema["api"]["release_date"],
        compatibility["upstream"]["telegram_bot_api"]["release_date"],
        "Bot API release date",
    )

    surface = compatibility["surface"]
    schema_categories: dict[str, int] = {}

    def collect_schema_categories(value: object) -> None:
        if not isinstance(value, dict):
            return
        category = value.get("category")
        if isinstance(category, str):
            schema_categories[category] = schema_categories.get(category, 0) + 1
        for child in value.get("children", []):
            collect_schema_categories(child)

    for item in schema["items"]:
        collect_schema_categories(item)
    require(schema_categories.get("types"), surface["schema_types"], "schema type count")
    require(
        schema_categories.get("methods"),
        surface["schema_methods"],
        "schema method count",
    )
    require(
        len(list((UPSTREAM / "aiogram" / "types").glob("*.py"))),
        surface["generated_python_type_modules"],
        "generated Python type module count",
    )
    require(
        len(list((UPSTREAM / "tests").rglob("test_*.py"))),
        surface["upstream_test_modules"],
        "upstream test module count",
    )
    manual_modules = [
        path
        for path in (UPSTREAM / "aiogram").rglob("*.py")
        if path.relative_to(UPSTREAM / "aiogram").parts[0]
        not in {"types", "methods", "enums"}
    ]
    require(
        len(manual_modules),
        surface["manual_python_modules"],
        "manual Python framework module count",
    )
    require(
        generated_count(ROOT / "src" / "types" / "generated.rs", "API_ENTITY_COUNT"),
        surface["butcher_type_entities"],
        "generated entity count",
    )
    require(
        generated_count(
            ROOT / "src" / "types" / "generated.rs",
            "MAPPED_PYTHON_TYPE_ANNOTATION_COUNT",
        ),
        surface["mapped_python_type_annotations"],
        "mapped Python type annotation count",
    )
    require(
        generated_count(ROOT / "src" / "types" / "generated.rs", "API_UNION_COUNT"),
        surface["aiogram_union_aliases"],
        "generated union count",
    )
    require(
        generated_count(ROOT / "src" / "enums" / "generated.rs", "API_ENUM_COUNT"),
        surface["aiogram_enums"],
        "generated enum count",
    )
    require(
        generated_count(ROOT / "src" / "methods" / "generated.rs", "API_METHOD_COUNT"),
        surface["schema_methods"],
        "generated method count",
    )
    require(
        generated_count(
            ROOT / "src" / "methods" / "generated.rs",
            "MAPPED_PYTHON_METHOD_ANNOTATION_COUNT",
        ),
        surface["mapped_python_method_annotations"],
        "mapped Python method annotation count",
    )
    require(
        generated_count(ROOT / "src" / "types" / "bound.rs", "BOUND_METHOD_COUNT"),
        surface["aiogram_bound_aliases"],
        "generated bound alias count",
    )
    print(
        "compatibility verified: "
        f"port {compatibility['port']['version']}, "
        f"aiogram {compatibility['upstream']['aiogram']['version']}@{commit[:12]}, "
        f"Bot API {schema['api']['version']}"
    )


if __name__ == "__main__":
    main()
