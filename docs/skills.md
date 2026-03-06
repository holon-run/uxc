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
- `skills/notion-mcp-skill`
  - Wrapper for Notion MCP workflows with OAuth and guarded-write guidance.
- `skills/discord-openapi-skill`
  - Wrapper for Discord REST workflows via UXC + OpenAPI schema mapping (`--schema-url`).
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

Replace `skills/deepwiki-mcp-skill` with `skills/context7-mcp-skill`, `skills/okx-mcp-skill`, `skills/notion-mcp-skill`, or `skills/discord-openapi-skill` as needed.

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

- Validate Discord wrapper docs when touched:

```bash
bash skills/discord-openapi-skill/scripts/validate.sh
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
