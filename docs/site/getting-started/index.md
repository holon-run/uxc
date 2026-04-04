# Getting Started

Use this section to get from install to a first successful call.

## Guides

- [Install](./install.md)
- [First Call](./first-call.md)
- [Import Existing MCP Config](./import-mcp-config.md)

## Core Flow

1. Discover operations with `uxc <host> -h`.
2. Inspect a specific operation with `uxc <host> <operation_id> -h`.
3. Invoke with structured arguments.

## Good First Targets

- OpenAPI: `petstore3.swagger.io/api/v3`
- GraphQL: `countries.trevorblades.com`
- MCP HTTP: `mcp.deepwiki.com/mcp`
- JSON-RPC: `fullnode.mainnet.sui.io`

<!-- INDEX:START -->

- [First Call](./first-call.md)
  Most first calls should start from discovery, not direct execution.
  <!-- mdorigin:index kind=article -->

- [Import Existing MCP Config](./import-mcp-config.md)
  If you already have MCP servers configured in tools like Cursor, Claude Code, Claude Desktop, VS Code, Codex, Windsurf, or OpenCode, UXC can import them into its own link and auth model.
  <!-- mdorigin:index kind=article -->

- [Install](./install.md)
  ```bash brew tap holon-run/homebrew-tap brew install uxc ```
  <!-- mdorigin:index kind=article -->

<!-- INDEX:END -->
