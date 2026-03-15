---
name: okx-exchange-websocket-skill
description: Subscribe to OKX public exchange WebSocket channels through UXC raw WebSocket mode for ticker, trade, book, and candle events with explicit subscribe frames.
---

# OKX Exchange WebSocket Skill

Use this skill to run OKX public exchange WebSocket channels through `uxc subscribe` raw WebSocket mode.

Reuse the `uxc` skill for generic runtime behavior, sink handling, and event-envelope parsing.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to the OKX public WebSocket endpoint.
- A writable sink path for NDJSON output.

## Scope

This skill covers OKX public exchange channels such as:

- tickers
- trades
- books and books5
- candle channels

This skill does **not** cover:

- OKX OnchainOS MCP
- private WebSocket login flows
- trading, account, or order-management channels
- REST API workflows

## Endpoint Model

Use the OKX public WebSocket endpoint:

- `wss://ws.okx.com:8443/ws/v5/public`

OKX public channels require a subscribe frame after connect, for example:

```json
{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}
```

## Core Workflow

1. Start a raw WebSocket subscription:
   - `uxc subscribe start wss://ws.okx.com:8443/ws/v5/public --transport websocket --init-frame '{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}' --sink file:$HOME/.uxc/subscriptions/okx-btcusdt-ticker.ndjson`
2. Inspect sink output:
   - `tail -n 5 $HOME/.uxc/subscriptions/okx-btcusdt-ticker.ndjson`
3. Query runtime status:
   - `uxc subscribe list`
   - `uxc subscribe status <job_id>`
4. Stop the job when finished:
   - `uxc subscribe stop <job_id>`

## Common Subscribe Frames

- tickers:
  - `{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}`
- trades:
  - `{"op":"subscribe","args":[{"channel":"trades","instId":"BTC-USDT"}]}`
- books5:
  - `{"op":"subscribe","args":[{"channel":"books5","instId":"BTC-USDT"}]}`
- candles:
  - `{"op":"subscribe","args":[{"channel":"candle1m","instId":"BTC-USDT"}]}`

## Runtime Validation

The following live raw WebSocket flow has been validated successfully through `uxc`:

- endpoint: `wss://ws.okx.com:8443/ws/v5/public`
- transport: `--transport websocket`
- init frame:
  - `{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}`

Observed sink behavior:

- initial `open`
- JSON `data` event for subscription acknowledgement
- repeated `data` events carrying ticker payloads under:
  - `arg.channel`
  - `arg.instId`
  - `data[0].last`
  - `data[0].bidPx`
  - `data[0].askPx`
  - `data[0].ts`

## Guardrails

- Keep automation on the JSON output envelope; do not use `--text`.
- Parse stable event fields first: `event_kind`, `data`, `meta`.
- Always pass `--transport websocket` for this skill.
- Public OKX channels do not require login. Do not mix this skill with private WebSocket auth flows.
- `--init-frame` is required because OKX public channels are multiplexed behind one endpoint.
- `instId` values use OKX instrument naming such as `BTC-USDT`.
- `uxc subscribe start ... --transport websocket` is the intended execution path for this skill; `uxc link` is not the main interface because channel selection lives in the subscribe frame.

## References

- Usage patterns:
  - `references/usage-patterns.md`
- OKX WebSocket API:
  - https://www.okx.com/docs-v5/en/
