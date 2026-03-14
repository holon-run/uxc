# Usage Patterns

This skill uses raw WebSocket subscriptions through `uxc subscribe start`.

## Trade Stream

```bash
uxc subscribe start \
  wss://stream.binance.com:443/ws/btcusdt@trade \
  --transport websocket \
  --sink file:$HOME/.uxc/subscriptions/binance-btcusdt-trade.ndjson
```

## Aggregate Trade Stream

```bash
uxc subscribe start \
  wss://stream.binance.com:443/ws/btcusdt@aggTrade \
  --transport websocket \
  --sink file:$HOME/.uxc/subscriptions/binance-btcusdt-aggtrade.ndjson
```

## Book Ticker Stream

```bash
uxc subscribe start \
  wss://stream.binance.com:443/ws/btcusdt@bookTicker \
  --transport websocket \
  --sink file:$HOME/.uxc/subscriptions/binance-btcusdt-bookticker.ndjson
```

## Combined Stream

```bash
uxc subscribe start \
  'wss://stream.binance.com:443/stream?streams=btcusdt@trade/ethusdt@trade' \
  --transport websocket \
  --sink file:$HOME/.uxc/subscriptions/binance-combined.ndjson
```

## Runtime Inspection

```bash
uxc subscribe list
uxc subscribe status sub_123
uxc subscribe stop sub_123
```

## Notes

- Binance stream names are lowercase.
- `uxc subscribe start ... --transport websocket` is the canonical execution path for this skill.
- Raw stream payloads arrive directly as market-event JSON.
- Combined stream payloads wrap each message with `stream` and `data`.
