---
name: context7
description: Query up-to-date library documentation and code examples using Context7 MCP. Use when you need current, version-specific documentation for npm packages, Python libraries, or other programming languages.
metadata:
  short-description: Query library docs via Context7 MCP
---

# Context7 Skill

Use this skill to query library documentation and code examples.

## Prerequisites

- `uxc` skill is installed (see [uxc skill](https://github.com/holon-run/uxc/tree/main/skills/uxc) for installation)
- Network access to `https://mcp.context7.com/mcp`

## Core Workflow

1. List available tools:
   - `uxc https://mcp.context7.com/mcp list`

2. Resolve a library name to get library ID:
   - `uxc https://mcp.context7.com/mcp resolve-library-id --input-json '{"libraryName":"react","query":"useState hook"}'`

3. Query documentation:
   - `uxc https://mcp.context7.com/mcp query-docs --input-json '{"libraryId":"/reactjs/react.dev","query":"how to use useState"}'`

## Available Tools

- **resolve-library-id**: Resolve a package/library name to Context7 library ID
- **query-docs**: Query documentation and code examples for a specific library

## Usage Examples

### Find React documentation

```bash
# First resolve the library
uxc https://mcp.context7.com/mcp resolve-library-id --input-json '{"libraryName":"react","query":"React useState hook"}'
```

### Query specific documentation

```bash
uxc https://mcp.context7.com/mcp query-docs --input-json '{"libraryId":"/reactjs/react.dev","query":"how to use useEffect"}'
```

### Query Node.js documentation

```bash
uxc https://mcp.context7.com/mcp resolve-library-id --input-json '{"libraryName":"node","query":"file system"}'
```

## Notes

- Requires library name first, then use the returned libraryId for queries
- Context7 provides version-specific, up-to-date documentation
- Supports npm packages, Python libraries, and more

## Reference Files

- Workflow details: `references/usage-patterns.md`
