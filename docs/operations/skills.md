# Skills Publish and Maintenance Log

This document is the publish, validation, and maintenance log for the skills in
this repository.

For the browsable skill catalog and category-based entrypoint, see
https://uxc.holon.run/skills/.

## Scope

Use this document for:

- validation and maintenance rules for skill docs
- manual publish workflow details
- ClawHub registry sync notes
- historical publish logs and release windows

Use https://uxc.holon.run/skills/ for:

- the browsable skill catalog
- category-based navigation
- recommended starting points

## Maintenance Rules

- Keep CLI examples in all skill docs aligned with current UXC syntax.
- If CLI semantics or output envelope changes, update:
  - `skills/uxc/SKILL.md`
  - `skills/uxc/references/*`
  - wrapper skill docs that include command snippets
- Run the relevant validators below when the corresponding skill docs are touched.

### Core

```bash
bash skills/uxc/scripts/validate.sh
bash skills/uxc-skill-creator/scripts/validate.sh
```

### Workspace and Messaging

```bash
bash skills/notion-mcp-skill/scripts/validate.sh
bash skills/notion-openapi-skill/scripts/validate.sh
bash skills/discord-openapi-skill/scripts/validate.sh
bash skills/slack-openapi-skill/scripts/validate.sh
bash skills/matrix-openapi-skill/scripts/validate.sh
bash skills/line-openapi-skill/scripts/validate.sh
bash skills/feishu-openapi-skill/scripts/validate.sh
bash skills/whatsapp-openapi-skill/scripts/validate.sh
bash skills/dingtalk-openapi-skill/scripts/validate.sh
```

### Data, Browser, and Web3

```bash
bash skills/playwright-mcp-skill/scripts/validate.sh
bash skills/chrome-devtools-mcp-skill/scripts/validate.sh
bash skills/context7-mcp-skill/scripts/validate.sh
bash skills/deepwiki-mcp-skill/scripts/validate.sh
bash skills/dune-mcp-skill/scripts/validate.sh
bash skills/thegraph-mcp-skill/scripts/validate.sh
bash skills/thegraph-token-mcp-skill/scripts/validate.sh
bash skills/etherscan-mcp-skill/scripts/validate.sh
bash skills/coinapi-openapi-skill/scripts/validate.sh
bash skills/dexscreener-openapi-skill/scripts/validate.sh
bash skills/helius-openapi-skill/scripts/validate.sh
bash skills/blocknative-openapi-skill/scripts/validate.sh
bash skills/near-jsonrpc-skill/scripts/validate.sh
bash skills/hive-mcp-skill/scripts/validate.sh
bash skills/nodit-openapi-skill/scripts/validate.sh
bash skills/mempool-space-openapi-skill/scripts/validate.sh
bash skills/alchemy-openapi-skill/scripts/validate.sh
bash skills/chainbase-openapi-skill/scripts/validate.sh
bash skills/blockscout-openapi-skill/scripts/validate.sh
bash skills/defillama-openapi-skill/scripts/validate.sh
bash skills/coingecko-openapi-skill/scripts/validate.sh
bash skills/moralis-openapi-skill/scripts/validate.sh
bash skills/binance-web3-openapi-skill/scripts/validate.sh
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

## ClawHub Publish Log (2026-03-15)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `slack-openapi-skill@1.0.0`
  - `discord-openapi-skill@1.0.1`
  - `feishu-openapi-skill@1.0.0`
  - `telegram-openapi-skill@1.0.0`
  - `matrix-openapi-skill@1.0.0`
  - `binance-spot-websocket-skill@1.0.0`
  - `bitquery-graphql-skill@1.0.1`
  - `binance-spot-openapi-skill@1.0.1`
- Notes:
  - The practical ClawHub limit still appears to be `5` new skills per hour.
  - Updates to already-published skills were accepted in the same window.
  - Workspace/README install indexes were updated together with this wave.

### Remaining publish queue after the 2026-03-15 window

- Next realtime / subscribe wave:
  - `bybit-openapi-skill`
  - `kraken-openapi-skill`
  - `kucoin-openapi-skill`
  - `bitget-openapi-skill`
  - `mexc-openapi-skill`
- Next market / data wave:
  - `upbit-openapi-skill`
  - `coingecko-openapi-skill`
  - `alchemy-openapi-skill`
  - `chainbase-openapi-skill`
  - `moralis-openapi-skill`
- Next data / MCP wave:
  - `blockscout-openapi-skill`
  - `coinapi-openapi-skill`
  - `birdeye-mcp-skill`
  - `lifi-mcp-skill`
  - `coinmarketcap-mcp-skill`
- Next MCP / DeFi wave:
  - `gate-mcp-skill`
  - `crypto-com-mcp-skill`
  - `defillama-openapi-skill`
  - `defillama-prices-openapi-skill`
  - `defillama-pro-openapi-skill`
- Final remaining wave:
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

## ClawHub Publish Log (2026-03-16)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `okx-exchange-websocket-skill@1.0.0`
  - `ethereum-jsonrpc-skill@1.0.0`
  - `sui-jsonrpc-skill@1.0.0`
  - `dingtalk-openapi-skill@1.0.0`
  - `coinbase-openapi-skill@1.0.0`

### Remaining publish queue after the 2026-03-16 window

- Next exchange coverage wave:
  - `upbit-openapi-skill`
  - `coingecko-openapi-skill`
  - `alchemy-openapi-skill`
  - `chainbase-openapi-skill`
  - `moralis-openapi-skill`
- Next data / MCP wave:
  - `blockscout-openapi-skill`
  - `coinapi-openapi-skill`
  - `birdeye-mcp-skill`
  - `lifi-mcp-skill`
  - `coinmarketcap-mcp-skill`
- Next MCP / DeFi wave:
  - `gate-mcp-skill`
  - `crypto-com-mcp-skill`
  - `defillama-openapi-skill`
  - `defillama-prices-openapi-skill`
  - `defillama-pro-openapi-skill`
- Final remaining wave:
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

## ClawHub Publish Log (2026-03-16, second window)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `bybit-openapi-skill@1.0.0`
  - `kraken-openapi-skill@1.0.0`
  - `kucoin-openapi-skill@1.0.0`
  - `bitget-openapi-skill@1.0.0`
  - `mexc-openapi-skill@1.0.0`

### Remaining publish queue after the 2026-03-16 second window

- Next market / data wave:
  - `blockscout-openapi-skill`
  - `coinapi-openapi-skill`
  - `birdeye-mcp-skill`
  - `lifi-mcp-skill`
  - `coinmarketcap-mcp-skill`
- Next MCP / DeFi wave:
  - `gate-mcp-skill`
  - `crypto-com-mcp-skill`
  - `defillama-openapi-skill`
  - `defillama-prices-openapi-skill`
  - `defillama-pro-openapi-skill`
- Final remaining wave:
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

## ClawHub Publish Log (2026-03-16, third window)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `upbit-openapi-skill@1.0.0`
  - `coingecko-openapi-skill@1.0.0`
  - `alchemy-openapi-skill@1.0.0`
  - `chainbase-openapi-skill@1.0.0`
  - `moralis-openapi-skill@1.0.0`

### Remaining publish queue after the 2026-03-16 third window

- Next data / MCP wave:
  - `chrome-devtools-mcp-skill`
  - `gate-mcp-skill`
  - `crypto-com-mcp-skill`
  - `defillama-openapi-skill`
  - `defillama-prices-openapi-skill`
  - `defillama-pro-openapi-skill`
- Final remaining wave:
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

### Rate-limit note

- Attempting the next 5-skill MCP/data wave immediately after the 2026-03-16 third window failed with:
  - `Rate limit: max 20 new skills per 24 hours. Please wait before publishing more.`
- The effective policy is therefore:
  - `max 5 new skills per hour`
  - `max 20 new skills per 24 hours`
- Resume from the `blockscout-openapi-skill` wave after the 24-hour window resets.

## ClawHub Publish Log (2026-03-17)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `blockscout-openapi-skill@1.0.0`
  - `coinapi-openapi-skill@1.0.0`
  - `birdeye-mcp-skill@1.0.0`
  - `lifi-mcp-skill@1.0.0`
  - `coinmarketcap-mcp-skill@1.0.0`
- Follow-up note:
  - `chrome-devtools-mcp-skill` was discovered as another unpublished skill and validated successfully.
  - Immediate publish attempt failed with:
    - `Rate limit: max 5 new skills per hour. Please wait before publishing more.`

### Remaining publish queue after the 2026-03-17 window

 - Next DeFi / messaging tail wave:
  - `defillama-pro-openapi-skill`
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

## ClawHub Publish Log (2026-03-17, second window)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `chrome-devtools-mcp-skill@1.0.0`
  - `gate-mcp-skill@1.0.0`
  - `crypto-com-mcp-skill@1.0.0`
  - `defillama-openapi-skill@1.0.0`
  - `defillama-prices-openapi-skill@1.0.0`

### Remaining publish queue after the 2026-03-17 second window

- Final remaining wave:
  - `defillama-pro-openapi-skill`
  - `defillama-yields-openapi-skill`
  - `goldrush-mcp-skill`
  - `line-openapi-skill`
  - `whatsapp-openapi-skill`

## ClawHub Publish Log (2026-03-17, final window)

- `clawhub whoami`: `jolestar`
- Published in the current rate-limit window:
  - `defillama-pro-openapi-skill@1.0.0`
  - `defillama-yields-openapi-skill@1.0.0`
  - `goldrush-mcp-skill@1.0.0`
  - `line-openapi-skill@1.0.0`
  - `whatsapp-openapi-skill@1.0.0`
- Post-publish verification:
  - `clawhub sync --dry-run` returned `Nothing to sync`
  - `clawhub inspect` for these new skills may temporarily return
    `Skill is hidden while security scan is pending`
  - this is expected and does not indicate a failed upload

## Current registry sync state

- As of the final 2026-03-17 window, all local `skills/*` packages are synced to ClawHub.

## ClawHub Publish Log (2026-03-19)

- `clawhub whoami`: `jolestar`
- Newly published in the current window:
  - `blocknative-openapi-skill@1.0.0`
  - `dexscreener-openapi-skill@1.0.0`
  - `helius-openapi-skill@1.0.0`
  - `hive-mcp-skill@1.0.0`
  - `mempool-space-openapi-skill@1.0.0`
- Updated in the same pass:
  - `feishu-openapi-skill@1.0.1`
  - `telegram-openapi-skill@1.0.1`
- Follow-up note:
  - continuing the same `clawhub sync` run hit:
    - `Rate limit: max 5 new skills per hour. Please wait before publishing more.`

### Remaining publish queue after the 2026-03-19 window

- `near-jsonrpc-skill@1.0.0`
- `nodit-openapi-skill@1.0.0`

## ClawHub Publish Log (2026-03-19, second window)

- `clawhub whoami`: `jolestar`
- Newly published in the current window:
  - `near-jsonrpc-skill@1.0.0`
  - `nodit-openapi-skill@1.0.0`
- Post-publish verification:
  - `clawhub sync --dry-run` returned `Nothing to sync`
  - all local `skills/*` packages are now synced to ClawHub again

## ClawHub Publish Log (2026-03-23)

- `clawhub whoami`: `jolestar`
- New local skill pending publish:
  - `notion-openapi-skill@1.0.0`
- Local validation passed:
  - `bash skills/notion-openapi-skill/scripts/validate.sh`
- Publish attempt with `clawhub sync --all --bump patch` failed on the ClawHub backend while preparing the new skill:
  - `This query or mutation function ran multiple paginated queries. Convex only supports a single paginated query in each function.`
- Post-failure verification:
  - `clawhub sync --dry-run` still reports `notion-openapi-skill  NEW`
  - `clawhub inspect notion-openapi-skill` returns `Skill not found`
- Current conclusion:
  - `notion-openapi-skill` is the only remaining unsynced local skill
  - the blocker is a ClawHub service-side publish error rather than a local packaging or validation issue

## ClawHub Publish Log (2026-03-24)

- Retried publish for the remaining new skill:
  - `notion-openapi-skill@1.0.0`
- Result:
  - publish succeeded via `clawhub sync --all --bump patch`
  - published id: `k976vm3gwkxctw86kcwrbgad5583hkxe`
- Post-publish verification:
  - `clawhub inspect notion-openapi-skill` now returns `Latest: 1.0.0`
  - `clawhub sync --dry-run` now returns `Nothing to sync`
- Current registry sync state:
  - all local `skills/*` packages are synced to ClawHub again
