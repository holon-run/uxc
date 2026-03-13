# Skills In This Repository

This repository ships one canonical skill for UXC (Universal X-Protocol CLI) and several official scenario wrappers.

## Skill Catalog

- `skills/uxc`
  - Canonical reusable execution layer for remote schema-exposed interfaces.
  - Other skills should call this skill for help-first discovery and operation execution patterns.
- `skills/deepwiki-mcp-skill`
  - Wrapper for DeepWiki MCP workflows.
- `skills/context7-mcp-skill`
  - Wrapper for Context7 MCP library documentation workflows.
- `skills/okx-mcp-skill`
  - Unified wrapper for OKX MCP workflows (token/market/wallet/swap).
- `skills/dune-mcp-skill`
  - Unified wrapper for Dune MCP workflows (table discovery, SQL query lifecycle, results, visualization).
- `skills/thegraph-mcp-skill`
  - Unified wrapper for The Graph Subgraph MCP workflows via stdio bridge (discovery, schema retrieval, deployment selection, GraphQL query execution, credential-driven env injection).
- `skills/thegraph-token-mcp-skill`
  - Unified wrapper for The Graph Token API MCP workflows (token metadata, wallet balances, transfers, holders, pools, market data).
- `skills/etherscan-mcp-skill`
  - Unified wrapper for Etherscan MCP workflows (address portfolio, token holders, contract lookup).
- `skills/notion-mcp-skill`
  - Wrapper for Notion MCP workflows with OAuth two-step login guidance and guarded-write guidance.
- `skills/discord-openapi-skill`
  - Wrapper for Discord REST workflows via UXC + link-persisted OpenAPI schema mapping (`uxc link --schema-url`).
- `skills/slack-openapi-skill`
  - Wrapper for Slack Web API messaging-core workflows via UXC + curated OpenAPI schema and bearer-token auth.
- `skills/matrix-openapi-skill`
  - Wrapper for Matrix Client-Server workflows via UXC + curated OpenAPI schema, homeserver-specific base URL, and bearer-token auth.
- `skills/line-openapi-skill`
  - Wrapper for LINE Messaging API messaging-core workflows via UXC + curated OpenAPI schema and bearer-token auth.
- `skills/feishu-openapi-skill`
  - Wrapper for Feishu or Lark IM workflows via UXC + curated OpenAPI schema and tenant-token bearer auth.
- `skills/whatsapp-openapi-skill`
  - Wrapper for WhatsApp Business Platform Cloud API workflows via UXC + curated OpenAPI schema and bearer-token auth.
- `skills/dingtalk-openapi-skill`
  - Wrapper for DingTalk v1.0 messaging workflows via UXC + curated OpenAPI schema and app-token bearer auth.
- `skills/defillama-openapi-skill`
  - Wrapper for DefiLlama Pro read-first analytics workflows via UXC + curated OpenAPI schema and path-templated API-key auth.
- `skills/coingecko-openapi-skill`
  - Wrapper for CoinGecko and GeckoTerminal read-first market data workflows via UXC + curated OpenAPI schema and API-key auth.
- `skills/binance-web3-openapi-skill`
  - Wrapper for Binance Web3 public market/research workflows via UXC + curated OpenAPI schema.
- `skills/binance-spot-openapi-skill`
  - Wrapper for Binance Spot market/account/order workflows via UXC + curated OpenAPI schema and signer-backed auth.
- `skills/uxc-skill-creator`
  - Creator skill for authoring new UXC-based wrapper skills with strict conventions.

## Recommended Usage Model

1. Install and rely on `skills/uxc` as the base capability.
2. Add wrapper skills only for repeated service-specific workflows.
3. Keep wrapper logic thin and delegate generic protocol execution to `skills/uxc`.
4. Use `skills/uxc-skill-creator` when creating or refactoring wrapper skills.

## Install For Codex

Install canonical `uxc` skill:

```bash
python ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo holon-run/uxc \
  --path skills/uxc
```

Install an official wrapper (example: deepwiki):

```bash
python ~/.codex/skills/.system/skill-installer/scripts/install-skill-from-github.py \
  --repo holon-run/uxc \
  --path skills/deepwiki-mcp-skill
```

Replace `skills/deepwiki-mcp-skill` with `skills/context7-mcp-skill`, `skills/okx-mcp-skill`, `skills/dune-mcp-skill`, `skills/thegraph-mcp-skill`, `skills/thegraph-token-mcp-skill`, `skills/etherscan-mcp-skill`, `skills/notion-mcp-skill`, `skills/discord-openapi-skill`, `skills/slack-openapi-skill`, `skills/matrix-openapi-skill`, `skills/line-openapi-skill`, `skills/feishu-openapi-skill`, `skills/whatsapp-openapi-skill`, `skills/dingtalk-openapi-skill`, `skills/coingecko-openapi-skill`, `skills/defillama-openapi-skill`, `skills/binance-web3-openapi-skill`, or `skills/binance-spot-openapi-skill` as needed.

After installation, restart Codex to load new skills.

## Maintenance Rules

- Keep CLI examples in all skill docs aligned with current UXC syntax.
- If CLI semantics or output envelope changes, update:
  - `skills/uxc/SKILL.md`
  - `skills/uxc/references/*`
  - wrapper skill docs that include command snippets
- Validate canonical skill docs:

```bash
bash skills/uxc/scripts/validate.sh
```

- Validate skill creator docs:

```bash
bash skills/uxc-skill-creator/scripts/validate.sh
```

- Validate Notion wrapper docs when touched:

```bash
bash skills/notion-mcp-skill/scripts/validate.sh
```

- Validate Etherscan wrapper docs when touched:

```bash
bash skills/etherscan-mcp-skill/scripts/validate.sh
```

- Validate Dune wrapper docs when touched:

```bash
bash skills/dune-mcp-skill/scripts/validate.sh
```

- Validate The Graph wrapper docs when touched:

```bash
bash skills/thegraph-mcp-skill/scripts/validate.sh
```

- Validate The Graph Token wrapper docs when touched:

```bash
bash skills/thegraph-token-mcp-skill/scripts/validate.sh
```

- Validate Discord wrapper docs when touched:

```bash
bash skills/discord-openapi-skill/scripts/validate.sh
```

- Validate Slack wrapper docs when touched:

```bash
bash skills/slack-openapi-skill/scripts/validate.sh
```

- Validate Matrix wrapper docs when touched:

```bash
bash skills/matrix-openapi-skill/scripts/validate.sh
```

- Validate LINE wrapper docs when touched:

```bash
bash skills/line-openapi-skill/scripts/validate.sh
```

- Validate Feishu wrapper docs when touched:

```bash
bash skills/feishu-openapi-skill/scripts/validate.sh
```

- Validate WhatsApp wrapper docs when touched:

```bash
bash skills/whatsapp-openapi-skill/scripts/validate.sh
```

- Validate DingTalk wrapper docs when touched:

```bash
bash skills/dingtalk-openapi-skill/scripts/validate.sh
```

- Validate DefiLlama wrapper docs when touched:

```bash
bash skills/defillama-openapi-skill/scripts/validate.sh
```

- Validate CoinGecko wrapper docs when touched:

```bash
bash skills/coingecko-openapi-skill/scripts/validate.sh
```

- Validate Binance Web3 wrapper docs when touched:

```bash
bash skills/binance-web3-openapi-skill/scripts/validate.sh
```

- Validate Binance Spot wrapper docs when touched:

```bash
bash skills/binance-spot-openapi-skill/scripts/validate.sh
```

## Manual Publish Workflow (GitHub Actions)

Skill publishing is intentionally manual to avoid registry rate-limit issues during high-frequency merges.

### Trigger

Run `.github/workflows/skills-publish.yml` with `workflow_dispatch`.

- `mode`:
  - `dry-run`: preview upload plan only
  - `publish`: dry-run first, then real sync
- `bump`:
  - `patch` (default)
  - `minor`
  - `major`

### Requirements

- Repository secret `CLAWHUB_TOKEN` must be configured.
- Workflow validates all `skills/*/scripts/validate.sh` before any sync.

### Behavior

- Sync command uses:

```bash
clawhub --no-input --workdir "$GITHUB_WORKSPACE" --dir skills sync --all
```

- `clawhub sync` compares local fingerprint with registry state:
  - unchanged skills: `Already synced`
  - changed/new skills: included in `To sync`
  - no changes: `Nothing to sync`

This makes repeated runs idempotent and safe.

## ClawHub Publish Log (2026-03-03)

- `clawhub whoami`: `jolestar`
- Published (1.0.0):
  - `playwright-mcp-skill`
  - `notion-mcp-skill`
  - `uxc`
  - `uxc-skill-creator`
  - `uxc-context7`
- ClawHub limit observed: max 5 new skills per hour.
- Next publish commands after rate-limit window:

```bash
clawhub publish skills/context7-mcp-skill --slug context7-mcp-skill --name "Context7 MCP Skill" --version 1.0.0
clawhub publish skills/deepwiki-mcp-skill --slug deepwiki-mcp-skill --name "DeepWiki MCP Skill" --version 1.0.0
```

## ClawHub Publish Log (2026-03-04)

- `clawhub whoami`: `jolestar`
- Published (1.0.0):
  - `okx-mcp-skill`
  - command: `clawhub publish skills/okx-mcp-skill --slug okx-mcp-skill --name "OKX MCP Skill" --version 1.0.0`
- Published (1.0.1):
  - `okx-mcp-skill`
  - reason: auth setup docs now require `--api-key-header OK-ACCESS-KEY` in initial credential command.
  - command: `clawhub publish skills/okx-mcp-skill --slug okx-mcp-skill --name "OKX MCP Skill" --version 1.0.1`

## ClawHub Publish Log (2026-03-07)

- Command used:

```bash
clawhub sync --dry-run
clawhub sync --all --bump patch
```

- `sync --dry-run` reported:
  - `dune-mcp-skill  NEW  (4 files)`
  - `etherscan-mcp-skill  NEW  (4 files)`
  - `thegraph-mcp-skill  NEW  (4 files)`
  - `thegraph-token-mcp-skill  NEW  (4 files)`
  - `uxc  UPDATE 1.0.1 → 1.0.2  (9 files)`
- Published:
  - `dune-mcp-skill@1.0.0`
  - `etherscan-mcp-skill@1.0.0`
  - `thegraph-mcp-skill@1.0.0`
  - `thegraph-token-mcp-skill@1.0.0`
  - `uxc@1.0.2`
- Notes:
  - `clawhub sync` compares current local skill fingerprints with registry state, not git history.
  - `uxc (9 files)` refers to the total file count in the skill package, not the number of changed files.
  - Immediately after publish, `clawhub inspect uxc` may return `Skill is hidden while security scan is pending`; `clawhub sync --dry-run` still reports `Already synced`, which confirms the upload succeeded.
