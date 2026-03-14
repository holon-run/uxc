# DefiLlama Yields API Skill - Usage Patterns

## Link Setup

```bash
command -v defillama-yields-openapi-cli
uxc link defillama-yields-openapi-cli https://yields.llama.fi \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-yields-openapi-skill/references/defillama-yields.openapi.json
defillama-yields-openapi-cli -h
```

This public skill does not require auth.

## Read Examples

```bash
# Discover yield pools
defillama-yields-openapi-cli get:/pools

# Read one pool's yield chart history
defillama-yields-openapi-cli get:/chart/{pool} pool=747c1d2a-c668-4682-b9f9-296708a3dd90
```

## Fallback Equivalence

- `defillama-yields-openapi-cli <operation> ...` is equivalent to
  `uxc https://yields.llama.fi --schema-url <defillama_yields_openapi_schema> <operation> ...`.
