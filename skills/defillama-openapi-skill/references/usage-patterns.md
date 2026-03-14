# DefiLlama Public API Skill - Usage Patterns

## Link Setup

```bash
command -v defillama-openapi-cli
uxc link defillama-openapi-cli https://api.llama.fi \
  --schema-url https://raw.githubusercontent.com/holon-run/uxc/main/skills/defillama-openapi-skill/references/defillama-public.openapi.json
defillama-openapi-cli -h
```

This public skill does not require auth.

## Read Examples

```bash
# List tracked protocols and their top-level TVL metrics
defillama-openapi-cli get:/protocols

# Read one protocol in detail
defillama-openapi-cli get:/protocol/{protocol} protocol=aave

# Read chain overview metrics
defillama-openapi-cli get:/v2/chains
```

## Fallback Equivalence

- `defillama-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.llama.fi --schema-url <defillama_openapi_schema> <operation> ...`.
