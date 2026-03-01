# Usage Patterns

All commands in this skill use the fixed stdio endpoint:
`npx -y @playwright/mcp@latest --headless --isolated`

This skill defaults to fixed link command `playwright-mcp-cli`.
Create it when missing:

```bash
command -v playwright-mcp-cli
uxc link playwright-mcp-cli "npx -y @playwright/mcp@latest --headless --isolated"
```

## Discover And Inspect

```bash
playwright-mcp-cli -h
playwright-mcp-cli browser_navigate -h
playwright-mcp-cli browser_snapshot -h
```

## Read-First Flow

Navigate first:

```bash
playwright-mcp-cli browser_navigate url=https://example.com
```

Capture accessibility snapshot:

```bash
playwright-mcp-cli browser_snapshot
```

## Action Flow (Confirm High-Impact Actions First)

Click by snapshot reference:

```bash
playwright-mcp-cli browser_click ref=e6
```

Run custom script:

```bash
playwright-mcp-cli browser_run_code code='await page.waitForTimeout(1000)'
```

## Bare JSON Positional Example

```bash
playwright-mcp-cli browser_navigate '{"url":"https://example.com"}'
```

## Output Parsing

Rely on envelope fields:
- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `playwright-mcp-cli <operation> ...` is equivalent to `uxc "npx -y @playwright/mcp@latest --headless --isolated" <operation> ...`.
- If link setup is temporarily unavailable, use the direct `uxc "<endpoint>" ...` form as fallback.
