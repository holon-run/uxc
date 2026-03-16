# Usage Patterns

This skill defaults to a live-browser stdio endpoint:
`npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics`

This skill defaults to the fixed link command `chrome-devtools-mcp-cli`.
Create it when missing:

```bash
command -v chrome-devtools-mcp-cli
uxc link chrome-devtools-mcp-cli "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics"
```

## Live Chrome Setup

Use this skill when your Chrome build exposes remote debugging settings at `chrome://inspect/#remote-debugging` and you have enabled them there.

```bash
chrome-devtools-mcp-cli -h
```

## Explicit Port-Based Attachment

Use this mode when you intentionally launch Chrome with `--remote-debugging-port=9222`.

```bash
command -v chrome-devtools-mcp-port
uxc link chrome-devtools-mcp-port "npx -y chrome-devtools-mcp@latest --browserUrl http://127.0.0.1:9222 --no-usage-statistics"
```

## Isolated Fallback

Use this mode when you do not have a debuggable Chrome instance available.

```bash
command -v chrome-devtools-mcp-isolated
uxc link chrome-devtools-mcp-isolated "npx -y chrome-devtools-mcp@latest --headless --isolated --no-usage-statistics"
```

## Discover And Inspect

```bash
chrome-devtools-mcp-cli -h
chrome-devtools-mcp-cli new_page -h
chrome-devtools-mcp-cli take_snapshot -h
chrome-devtools-mcp-cli list_network_requests -h
chrome-devtools-mcp-cli lighthouse_audit -h
```

## Read-First Flow

Open a page:

```bash
chrome-devtools-mcp-cli new_page url=https://example.com
```

Capture a text snapshot:

```bash
chrome-devtools-mcp-cli take_snapshot verbose=true
```

Inspect network traffic:

```bash
chrome-devtools-mcp-cli list_network_requests pageSize=20
```

Inspect console messages:

```bash
chrome-devtools-mcp-cli list_console_messages
```

Run a Lighthouse audit:

```bash
chrome-devtools-mcp-cli lighthouse_audit
```

## Action Flow (Confirm High-Impact Actions First)

Click a page element by snapshot uid:

```bash
chrome-devtools-mcp-cli click uid=6_1
```

Fill an input:

```bash
chrome-devtools-mcp-cli fill uid=7_1 value='search text'
```

Evaluate JavaScript:

```bash
chrome-devtools-mcp-cli evaluate_script function='() => document.title'
```

## Bare JSON Positional Example

```bash
chrome-devtools-mcp-cli new_page '{"url":"https://example.com","timeout":10000}'
```

## Output Parsing

Rely on envelope fields:
- Success: `ok == true`, consume `data`
- Failure: `ok == false`, inspect `error.code` and `error.message`

## Fallback Equivalence

- `chrome-devtools-mcp-cli <operation> ...` is equivalent to `uxc "npx -y chrome-devtools-mcp@latest --autoConnect --no-usage-statistics" <operation> ...`.
- If link setup is temporarily unavailable, use the direct `uxc "<endpoint>" ...` form as fallback.
