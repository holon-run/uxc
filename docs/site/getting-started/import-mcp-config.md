# Import Existing MCP Config

If you already have MCP servers configured in tools like Cursor, Claude Code,
Claude Desktop, VS Code, Codex, Windsurf, or OpenCode, UXC can import them into
its own link and auth model.

## Dry Run First

Preview what UXC will import:

```bash
uxc config import mcp --dry-run
```

This checks common MCP config locations automatically and shows the normalized
import plan without creating links or auth assets.

## Import From A Specific Source

Import from an explicit preset or config file:

```bash
uxc config import mcp --from cursor
uxc config import mcp --from codex
uxc config import mcp --from ~/.cursor/mcp.json
```

Supported source presets in v1:

- `auto`
- `cursor`
- `claude-code`
- `claude-desktop`
- `vscode`
- `codex`
- `windsurf`
- `opencode`

## Skip Link Creation

If you only want normalized auth assets and not generated link commands:

```bash
uxc config import mcp --from auto --skip-create-links
```

## Control The Link Directory

Import and create links under a specific directory:

```bash
uxc config import mcp --from auto --link-dir ~/.local/bin
```

## What Gets Imported

UXC treats external MCP configs as input sources and converts them into UXC
assets:

- normalized host definitions
- generated link commands
- reusable auth assets when credential-like placeholders are detected

This keeps subsequent usage on the standard UXC path instead of repeatedly
consuming external config files at runtime.

## Next Step

After import, validate a generated link:

```bash
<link_name> -h
```

Then continue with the normal discovery-first workflow:

```bash
<link_name> <operation_id> -h
<link_name> <operation_id> key=value
```
