---
name: binance-spot-websocket-skill
description: Subscribe to Binance Spot public market streams through UXC raw WebSocket support for trades, book ticker, depth, and ticker events with stream-specific guardrails.
---

# Binance Spot WebSocket Skill

Use this skill to run Binance Spot public market streams through `uxc subscribe` raw WebSocket mode.

Reuse the `uxc` skill for generic runtime behavior, sink handling, and event-envelope parsing.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to Binance Spot public WebSocket streams.
- A writable sink path for NDJSON output.

## Scope

This skill covers Binance Spot public market streams such as:

- trade events
- aggregate trade events
- book ticker updates
- ticker updates
- depth updates

This skill does **not** cover:

- Binance Spot REST/OpenAPI workflows
- private user data streams
- signed WebSocket API methods
- margin, wallet, futures, or other non-Spot product families

## Endpoint Model

Binance Spot public market streams use raw WebSocket endpoints.

- preferred base: `wss://stream.binance.com:443`
- raw stream form: `wss://stream.binance.com:443/ws/<streamName>`
- combined stream form: `wss://stream.binance.com:443/stream?streams=<stream1>/<stream2>`

Important:

- stream names are lowercase
- connections are valid for up to 24 hours
- raw stream payloads arrive directly as JSON objects
- combined stream payloads arrive as `{"stream":"...","data":{...}}`

## Core Workflow

1. Start a subscription directly with `uxc subscribe start`:
   - `uxc subscribe start wss://stream.binance.com:443/ws/btcusdt@trade --transport websocket --sink file:$HOME/.uxc/subscriptions/binance-btcusdt-trade.ndjson`
2. Inspect the sink output:
   - `tail -n 5 $HOME/.uxc/subscriptions/binance-btcusdt-trade.ndjson`
3. Query runtime status:
   - `uxc subscribe list`
   - `uxc subscribe status <job_id>`
4. Stop the job when finished:
   - `uxc subscribe stop <job_id>`

## Common Stream Targets

- raw trade stream:
  - `btcusdt@trade`
- aggregate trade stream:
  - `btcusdt@aggTrade`
- book ticker stream:
  - `btcusdt@bookTicker`
- mini ticker stream:
  - `btcusdt@miniTicker`
- rolling ticker stream:
  - `btcusdt@ticker`
- depth stream:
  - `btcusdt@depth`

## Runtime Validation

The following live raw WebSocket flow was validated successfully through `uxc`:

- endpoint: `wss://stream.binance.com:443/ws/btcusdt@trade`
- transport: `--transport websocket`
- sink output:
  - `open`
  - repeated `data` events with Binance `trade` payloads
  - `closed` after stop

Observed event fields included:

- `e: "trade"`
- `s: "BTCUSDT"`
- `p`
- `q`
- `t`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable event fields first: `event_kind`, `data`, `meta`.
- Use `wss://stream.binance.com:443` as the default public stream host; it validated more reliably than `:9443` in recent runtime checks.
- Stream names must be lowercase.
- This skill is read-only. Do not describe it as account, order, or signed WebSocket support.
- Combined streams are useful when one job should emit multiple markets into one sink, but downstream parsing must handle `stream` wrappers.
- `uxc subscribe start ... --transport websocket` is the intended execution path for this skill; `uxc link` is not the main interface because stream identity is encoded directly in the endpoint path.

## References

- Usage patterns:
  - `references/usage-patterns.md`
- Binance Spot WebSocket Streams:
  - https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams
