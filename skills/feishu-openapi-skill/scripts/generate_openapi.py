#!/usr/bin/env python3
"""Generate the Feishu/Lark OpenAPI schema from official CLI metadata.

The upstream Feishu/Lark endpoint exposes a custom `protocol=meta` format, not a
standard OpenAPI document. This script converts the selected metadata services
into OpenAPI 3 and then applies a small curated overlay for operations that are
missing from metadata or need UXC-specific guardrails.
"""

from __future__ import annotations

import argparse
import copy
import json
import sys
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[3]
SKILL_DIR = ROOT / "skills" / "feishu-openapi-skill"
DEFAULT_OUTPUT = SKILL_DIR / "references" / "feishu-im.openapi.json"
DEFAULT_OVERLAY = SKILL_DIR / "references" / "feishu-openapi.overlay.json"

META_ENDPOINTS = {
    "feishu": "https://open.feishu.cn/api/tools/open/api_definition",
    "lark": "https://open.larksuite.com/api/tools/open/api_definition",
}


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as fp:
        return json.load(fp)


def fetch_meta(brand: str, client_version: str) -> dict[str, Any]:
    base = META_ENDPOINTS[brand]
    query = urllib.parse.urlencode(
        {"protocol": "meta", "client_version": client_version}
    )
    with urllib.request.urlopen(f"{base}?{query}", timeout=15) as resp:
        envelope = json.load(resp)
    if envelope.get("msg") != "succeeded":
        raise RuntimeError(f"unexpected metadata response: {envelope!r}")
    data = envelope.get("data")
    if not isinstance(data, dict) or not data.get("services"):
        raise RuntimeError("metadata response did not include services")
    return data


def deep_merge(base: Any, overlay: Any) -> Any:
    if isinstance(base, dict) and isinstance(overlay, dict):
        merged = copy.deepcopy(base)
        for key, value in overlay.items():
            if key in merged:
                merged[key] = deep_merge(merged[key], value)
            else:
                merged[key] = copy.deepcopy(value)
        return merged
    return copy.deepcopy(overlay)


def openapi_path(service_path: str, method_path: str) -> str:
    path = f"{service_path.rstrip('/')}/{method_path.lstrip('/')}"
    if path.startswith("/open-apis/"):
        path = path[len("/open-apis") :]
    return path


def schema_name(value: str) -> str:
    parts = [
        part
        for part in (
            value.replace(".", " ")
            .replace("_", " ")
            .replace("-", " ")
            .replace("/", " ")
            .split()
        )
        if part
    ]
    return "".join(part[:1].upper() + part[1:] for part in parts)


def convert_type(value: str | None) -> str:
    if value in {"int", "int32", "int64", "integer"}:
        return "integer"
    if value in {"float", "double", "number"}:
        return "number"
    if value in {"bool", "boolean"}:
        return "boolean"
    if value == "array":
        return "array"
    if value == "object":
        return "object"
    if value == "file":
        return "string"
    return "string"


def convert_field(field: dict[str, Any]) -> dict[str, Any]:
    field_type = convert_type(str(field.get("type") or "string"))
    schema: dict[str, Any] = {"type": field_type}

    if field.get("type") == "file":
        schema["format"] = "binary"

    description = field.get("description")
    if isinstance(description, str) and description:
        schema["description"] = description

    example = field.get("example")
    if example not in (None, ""):
        schema["example"] = example

    options = field.get("options")
    if isinstance(options, list) and options:
        enum_values = [
            option.get("value")
            for option in options
            if isinstance(option, dict) and option.get("value") is not None
        ]
        if enum_values:
            schema["enum"] = enum_values

    if field_type == "array":
        properties = field.get("properties")
        if isinstance(properties, dict) and properties:
            schema["items"] = convert_object_schema(properties)
        else:
            schema["items"] = {"type": "string"}
    elif field_type == "object":
        properties = field.get("properties")
        if isinstance(properties, dict) and properties:
            schema = deep_merge(schema, convert_object_schema(properties))

    return schema


def convert_object_schema(fields: dict[str, Any]) -> dict[str, Any]:
    properties: dict[str, Any] = {}
    required: list[str] = []
    for name in sorted(fields):
        field = fields[name]
        if not isinstance(field, dict):
            continue
        properties[name] = convert_field(field)
        if field.get("required") is True:
            required.append(name)
    schema: dict[str, Any] = {"type": "object", "properties": properties}
    if required:
        schema["required"] = required
    return schema


def convert_parameter(name: str, spec: dict[str, Any]) -> dict[str, Any]:
    location = spec.get("location") or "query"
    if location == "body":
        location = "query"
    parameter = {
        "name": name,
        "in": location,
        "required": bool(spec.get("required") or location == "path"),
        "schema": convert_field(spec),
    }
    description = spec.get("description")
    if isinstance(description, str) and description:
        parameter["description"] = description
    return parameter


def request_body_for(method: dict[str, Any]) -> dict[str, Any] | None:
    fields = method.get("requestBody")
    if not isinstance(fields, dict) or not fields:
        return None
    schema = convert_object_schema(fields)
    has_file = any(
        isinstance(field, dict) and field.get("type") == "file"
        for field in fields.values()
    )
    media_type = "multipart/form-data" if has_file else "application/json"
    return {
        "required": any(
            isinstance(field, dict) and field.get("required") is True
            for field in fields.values()
        ),
        "content": {media_type: {"schema": schema}},
    }


def response_for(method: dict[str, Any]) -> dict[str, Any]:
    fields = method.get("responseBody")
    if not isinstance(fields, dict) or not fields:
        return {"description": "Feishu/Lark OpenAPI response"}
    return {
        "description": "Feishu/Lark OpenAPI response",
        "content": {
            "application/json": {
                "schema": convert_object_schema(fields),
            }
        },
    }


def operation_id(service: str, resource: str, method: str) -> str:
    return service + schema_name(resource) + schema_name(method)


def convert_method(
    service_name: str,
    resource_name: str,
    method_name: str,
    method: dict[str, Any],
) -> dict[str, Any]:
    op: dict[str, Any] = {
        "operationId": operation_id(service_name, resource_name, method_name),
        "summary": method.get("summary") or method.get("description") or method_name,
        "responses": {"200": response_for(method)},
        "x-feishu-source": "api_definition:meta",
    }

    description = method.get("description")
    if isinstance(description, str) and description:
        op["description"] = description

    parameters = method.get("parameters")
    if isinstance(parameters, dict) and parameters:
        op["parameters"] = [
            convert_parameter(name, parameters[name]) for name in sorted(parameters)
        ]

    body = request_body_for(method)
    if body is not None:
        op["requestBody"] = body

    for src_key, dst_key in (
        ("scopes", "x-feishu-scopes"),
        ("accessTokens", "x-feishu-access-tokens"),
        ("docUrl", "x-feishu-doc-url"),
    ):
        value = method.get(src_key)
        if value:
            op[dst_key] = value

    return op


def convert_meta(meta: dict[str, Any], selected_services: set[str]) -> dict[str, Any]:
    paths: dict[str, Any] = {}
    services = meta.get("services")
    if not isinstance(services, list):
        return paths

    for service in services:
        if not isinstance(service, dict):
            continue
        service_name = service.get("name")
        if service_name not in selected_services:
            continue
        service_path = service.get("servicePath")
        resources = service.get("resources")
        if not isinstance(service_path, str) or not isinstance(resources, dict):
            continue
        for resource_name in sorted(resources):
            resource = resources[resource_name]
            if not isinstance(resource, dict):
                continue
            methods = resource.get("methods")
            if not isinstance(methods, dict):
                continue
            for method_name in sorted(methods):
                method = methods[method_name]
                if not isinstance(method, dict):
                    continue
                http_method = str(method.get("httpMethod") or "").lower()
                method_path = method.get("path")
                if not http_method or not isinstance(method_path, str):
                    continue
                path = openapi_path(service_path, method_path)
                paths.setdefault(path, {})[http_method] = convert_method(
                    str(service_name), str(resource_name), str(method_name), method
                )
    return paths


def build_schema(meta: dict[str, Any], overlay: dict[str, Any], services: set[str]) -> dict[str, Any]:
    schema: dict[str, Any] = {
        "openapi": "3.0.3",
        "info": {
            "title": "Feishu / Lark IM API (Curated)",
            "version": "1.0.0",
            "description": "Curated Feishu / Lark IM, bot identity, and contact surface for UXC messaging workflows.",
        },
        "servers": [
            {"url": "https://open.feishu.cn/open-apis"},
            {"url": "https://open.larksuite.com/open-apis"},
        ],
        "security": [{"FeishuBearerAuth": []}],
        "paths": convert_meta(meta, services),
        "components": {"schemas": {}},
        "x-feishu-meta-version": meta.get("version"),
    }
    return deep_merge(schema, overlay)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--brand", choices=sorted(META_ENDPOINTS), default="feishu")
    parser.add_argument("--client-version", default="uxc-generator")
    parser.add_argument("--meta-json", type=Path)
    parser.add_argument("--overlay", type=Path, default=DEFAULT_OVERLAY)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--service",
        action="append",
        default=["im"],
        help="metadata service to include; can be repeated",
    )
    args = parser.parse_args()

    meta = load_json(args.meta_json) if args.meta_json else fetch_meta(args.brand, args.client_version)
    if "data" in meta and isinstance(meta["data"], dict):
        meta = meta["data"]

    overlay = load_json(args.overlay)
    schema = build_schema(meta, overlay, set(args.service))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as fp:
        json.dump(schema, fp, ensure_ascii=False, indent=2, sort_keys=True)
        fp.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
