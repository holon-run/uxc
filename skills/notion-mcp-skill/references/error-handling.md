# Error Handling

## Scope

This file keeps Notion-specific failure notes only.
For canonical error taxonomy and OAuth recovery playbooks, use `uxc` skill:
- section: `Failure handling and retry strategy`
- file name in `$uxc`: `references/error-handling.md`

## Notion-Specific Failure Notes

1. If first real call returns `invalid_token`:
1. Check for duplicate endpoint bindings (`uxc auth binding list`).
2. Confirm the binding that currently matches:
   - `uxc auth binding match mcp.notion.com/mcp`
3. If multiple candidates exist, verify each candidate with explicit credential:
   - `uxc --auth <credential_id> mcp.notion.com/mcp <same_read_operation> ...`
4. Remove only the binding(s) confirmed stale/invalid.
5. Retry the original read call that failed.

2. When `notion-update-page` signals deletion risk:
1. Do not retry automatically with permissive flags.
2. Show what would be deleted.
3. Ask for explicit confirmation before executing destructive change.
