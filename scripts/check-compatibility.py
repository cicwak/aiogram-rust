#!/usr/bin/env python3
from __future__ import annotations

import ast
import hashlib
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


def manual_public_surface(
    root: Path,
) -> tuple[set[str], set[str], set[str], dict[str, list[str]], str]:
    classes: set[str] = set()
    functions: set[str] = set()
    methods: set[str] = set()
    symbols_by_module: dict[str, list[str]] = {}
    for path in sorted(root.rglob("*.py")):
        relative = path.relative_to(root)
        if relative.parts[0] in {"types", "methods", "enums"}:
            continue
        module = ".".join(relative.with_suffix("").parts)
        module_symbols: list[str] = []
        for node in ast.parse(path.read_text()).body:
            if isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
                class_name = f"{module}.{node.name}"
                classes.add(class_name)
                module_symbols.append(f"class:{class_name}")
                for member in node.body:
                    if isinstance(member, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        if not member.name.startswith("_"):
                            method_name = f"{class_name}.{member.name}"
                            methods.add(method_name)
                            module_symbols.append(f"method:{method_name}")
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if not node.name.startswith("_"):
                    function_name = f"{module}.{node.name}"
                    functions.add(function_name)
                    module_symbols.append(f"function:{function_name}")
        if module_symbols:
            symbols_by_module[module] = sorted(set(module_symbols))
    inventory = [
        *(f"class:{name}" for name in sorted(classes)),
        *(f"function:{name}" for name in sorted(functions)),
        *(f"method:{name}" for name in sorted(methods)),
    ]
    digest = hashlib.sha256(("\n".join(inventory) + "\n").encode()).hexdigest()
    return classes, functions, methods, symbols_by_module, digest


def manual_api_coverage(
    symbols_by_module: dict[str, list[str]],
) -> tuple[int, str, dict[str, int]]:
    route_path = ROOT / "compatibility" / "manual-api-routes.toml"
    document = tomllib.loads(route_path.read_text())
    routes = document.get("route", [])
    if not isinstance(routes, list):
        raise RuntimeError(f"route must be an array of tables in {route_path}")

    by_module: dict[str, dict[str, object]] = {}
    modes = {"native", "semantic", "language"}
    for route in routes:
        if not isinstance(route, dict):
            raise RuntimeError(f"invalid route in {route_path}: {route!r}")
        module = route.get("python_module")
        mode = route.get("mode")
        rust = route.get("rust")
        evidence = route.get("evidence")
        if not isinstance(module, str) or not module:
            raise RuntimeError(f"route is missing python_module in {route_path}")
        if module in by_module:
            raise RuntimeError(f"duplicate manual API route for {module}")
        if mode not in modes:
            raise RuntimeError(f"invalid coverage mode for {module}: {mode!r}")
        if not isinstance(rust, list) or not rust or not all(
            isinstance(value, str) and value for value in rust
        ):
            raise RuntimeError(f"route {module} needs non-empty Rust symbols")
        if not isinstance(evidence, list) or not evidence or not all(
            isinstance(value, str) and value for value in evidence
        ):
            raise RuntimeError(f"route {module} needs non-empty evidence paths")
        for value in evidence:
            evidence_path = ROOT / value.split("#", 1)[0]
            if not evidence_path.exists():
                raise RuntimeError(
                    f"manual API evidence for {module} does not exist: {value}"
                )
        by_module[module] = route

    require(
        set(by_module),
        set(symbols_by_module),
        "manual Python module coverage",
    )
    inventory: list[str] = []
    mode_counts = {mode: 0 for mode in sorted(modes)}
    for module, symbols in sorted(symbols_by_module.items()):
        route = by_module[module]
        mode = str(route["mode"])
        rust = ",".join(sorted(str(value) for value in route["rust"]))
        evidence = ",".join(sorted(str(value) for value in route["evidence"]))
        for symbol in symbols:
            inventory.append(f"{symbol}=>{mode}|{rust}|{evidence}")
            mode_counts[mode] += 1
    digest = hashlib.sha256(("\n".join(inventory) + "\n").encode()).hexdigest()
    return len(inventory), digest, mode_counts


def method_default_surface(root: Path) -> tuple[set[str], str]:
    mappings: set[str] = set()
    for path in sorted(root.glob("*.py")):
        for node in ast.parse(path.read_text()).body:
            if not isinstance(node, ast.ClassDef):
                continue
            for member in node.body:
                if not isinstance(member, ast.AnnAssign):
                    continue
                if not isinstance(member.target, ast.Name) or member.value is None:
                    continue
                for value in ast.walk(member.value):
                    if not isinstance(value, ast.Call):
                        continue
                    if not isinstance(value.func, ast.Name) or value.func.id != "Default":
                        continue
                    if not value.args or not isinstance(value.args[0], ast.Constant):
                        continue
                    property_name = value.args[0].value
                    if isinstance(property_name, str):
                        mappings.add(
                            f"{node.name}.{member.target.id}={property_name}"
                        )
                    break
    digest = hashlib.sha256(
        ("\n".join(sorted(mappings)) + "\n").encode()
    ).hexdigest()
    return mappings, digest


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
    (
        public_classes,
        public_functions,
        public_methods,
        symbols_by_module,
        public_surface_digest,
    ) = (
        manual_public_surface(UPSTREAM / "aiogram")
    )
    require(
        len(public_classes),
        surface["manual_python_public_classes"],
        "manual Python public class count",
    )
    require(
        len(public_functions),
        surface["manual_python_public_functions"],
        "manual Python public function count",
    )
    require(
        len(public_methods),
        surface["manual_python_public_methods"],
        "manual Python public method count",
    )
    require(
        public_surface_digest,
        surface["manual_python_public_surface_sha256"],
        "manual Python public surface fingerprint",
    )
    covered_symbols, coverage_digest, coverage_modes = manual_api_coverage(
        symbols_by_module
    )
    require(
        covered_symbols,
        surface["manual_python_covered_symbols"],
        "manual Python covered symbol count",
    )
    require(
        coverage_digest,
        surface["manual_python_coverage_sha256"],
        "manual Python coverage fingerprint",
    )
    for mode, count in coverage_modes.items():
        require(
            count,
            surface[f"manual_python_{mode}_symbols"],
            f"manual Python {mode} symbol count",
        )
    method_defaults, method_defaults_digest = method_default_surface(
        UPSTREAM / "aiogram" / "methods"
    )
    require(
        len(method_defaults),
        surface["mapped_python_method_defaults"],
        "upstream Python method default count",
    )
    require(
        method_defaults_digest,
        surface["python_method_defaults_sha256"],
        "upstream Python method default fingerprint",
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
        generated_count(
            ROOT / "src" / "methods" / "generated.rs",
            "MAPPED_PYTHON_METHOD_DEFAULT_COUNT",
        ),
        surface["mapped_python_method_defaults"],
        "mapped Python method default count",
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
        f"Bot API {schema['api']['version']}; "
        f"manual API {covered_symbols}/{covered_symbols} "
        f"({', '.join(f'{mode}={count}' for mode, count in coverage_modes.items())})"
    )


if __name__ == "__main__":
    main()
