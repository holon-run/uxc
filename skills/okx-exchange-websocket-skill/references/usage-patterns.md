# Usage Patterns

This skill uses raw WebSocket subscriptions through `uxc subscribe start` with explicit OKX subscribe frames.

## Ticker Channel

```bash
uxc subscribe start \
  wss://ws.okx.com:8443/ws/v5/public \
  --transport websocket \
  --init-frame '{"op":"subscribe","args":[{"channel":"tickers","instId":"BTC-USDT"}]}' \
  --sink file:$HOME/.uxc/subscriptions/okx-btcusdt-ticker.ndjson
```

## Trades Channel

```bash
uxc subscribe start \
  wss://ws.okx.com:8443/ws/v5/public \
  --transport websocket \
  --init-frame '{"op":"subscribe","args":[{"channel":"trades","instId":"BTC-USDT"}]}' \
  --sink file:$HOME/.uxc/subscriptions/okx-btcusdt-trades.ndjson
```

## Books5 Channel

```bash
uxc subscribe start \
  wss://ws.okx.com:8443/ws/v5/public \
  --transport websocket \
  --init-frame '{"op":"subscribe","args":[{"channel":"books5","instId":"BTC-USDT"}]}' \
  --sink file:$HOME/.uxc/subscriptions/okx-btcusdt-books5.ndjson
```

## Runtime Inspection

```bash
uxc subscribe list
uxc subscribe status sub_123
uxc subscribe stop sub_123
```

## Notes

- `--init-frame` is required for channel selection.
- Subscription acknowledgements arrive as normal JSON `data` events.
- Realtime payloads also arrive as JSON `data` events in the same sink.
