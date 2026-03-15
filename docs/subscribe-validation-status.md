# Subscribe Validation Status

This document tracks the current validation status of `uxc subscribe` across:

- local test coverage
- real provider validation
- known gaps and follow-up issues

It is intended to answer two questions quickly:

1. Which subscription modes are implemented and covered by automated tests?
2. Which real provider skills have been validated successfully, and which still have compatibility gaps?

## Current Subscription Surface

`uxc subscribe` currently supports:

- stream mode
  - raw HTTP streams
  - explicit raw WebSocket streams
  - GraphQL subscriptions over WebSocket
  - JSON-RPC pubsub over WebSocket
  - MCP `resources.subscribe`
- poll mode
  - repeated invocation of a normal operation
  - checkpoint persistence
  - item-key, watermark, content-hash, and cursor-based progression
  - item-derived cursor progression with transform support

Core implementation:

- [src/main.rs](../src/main.rs)
- [src/daemon.rs](../src/daemon.rs)
- [src/subscription_websocket.rs](../src/subscription_websocket.rs)
- [src/subscription_poll.rs](../src/subscription_poll.rs)

## Automated Local Coverage

The local E2E suite currently validates these subscription paths:

- HTTP stream subscribe
- raw WebSocket subscribe
- poll subscribe
- GraphQL subscribe
- JSON-RPC subscribe
- MCP stdio resource subscribe
- MCP HTTP resource subscribe

Relevant tests:

- [tests/local_e2e_test.rs](../tests/local_e2e_test.rs)
- [tests/subscribe_cli_test.rs](../tests/subscribe_cli_test.rs)

Most recent local verification run:

- `cargo test --test subscribe_cli_test -- --test-threads=1`
- `cargo test --features test-server --test local_e2e_test -- --test-threads=1`

Observed result:

- `subscribe_cli_test`: passed
- `local_e2e_test`: `47 passed`

## Real Provider Validation

### Verified

#### Binance Spot WebSocket

Skill:

- [skills/binance-spot-websocket-skill/SKILL.md](../skills/binance-spot-websocket-skill/SKILL.md)

Validation result:

- verified against the live Binance Spot public stream host
- raw WebSocket subscribe on `wss://stream.binance.com:443/ws/btcusdt@trade` succeeded
- `open` event was emitted
- repeated `data` events carried live Binance `trade` payloads
- `closed` event was emitted after explicit stop

Observed payload fields included:

- `e: "trade"`
- `s: "BTCUSDT"`
- `p`
- `q`
- `t`

Status:

- **validated successfully**

#### OKX Exchange WebSocket

Skill:

- [skills/okx-exchange-websocket-skill/SKILL.md](../skills/okx-exchange-websocket-skill/SKILL.md)

Validation result:

- verified against the live OKX public WebSocket endpoint
- raw WebSocket subscribe on `wss://ws.okx.com:8443/ws/v5/public` succeeded
- explicit subscribe frame for `tickers` / `BTC-USDT` was accepted
- sink contained both subscription acknowledgement and repeated ticker payloads

Observed payload fields included:

- `arg.channel`
- `arg.instId`
- `data[0].last`
- `data[0].bidPx`
- `data[0].askPx`
- `data[0].ts`

Status:

- **validated successfully**

#### Telegram Bot API

Skill:

- [skills/telegram-openapi-skill/SKILL.md](../skills/telegram-openapi-skill/SKILL.md)

Validation result:

- verified against the real Telegram Bot API
- `get:/getMe` succeeded
- `get:/getWebhookInfo` succeeded
- daemon-backed polling subscribe on `post:/getUpdates` succeeded
- real `data` events were emitted for incoming bot messages
- `poll` events recorded fetched/emitted/skipped counts
- `checkpoint` events were emitted
- repeated polls skipped already-consumed updates after checkpoint advancement

Important runtime note:

- Telegram allows only one active `getUpdates` consumer per bot token
- concurrent polling returns HTTP 409

Status:

- **validated successfully**

Related work:

- [#269](https://github.com/holon-run/uxc/issues/269) closed

### Verified

#### Bitquery GraphQL

Skill:

- [skills/bitquery-graphql-skill/SKILL.md](../skills/bitquery-graphql-skill/SKILL.md)

Validation result:

- schema discovery succeeded
- `subscription/*` roots are visible in introspection
- OAuth credential and auth binding were valid
- normal `query/*` execution succeeded
- GraphQL subscription runs now succeed when the subscription shape matches Bitquery's runtime expectations
- live `subscription/EVM` validation succeeded against `network=bsc` with `mempool=true`
- sink emitted `open` followed by repeated `data` events containing real transfer payloads

Observed runtime notes:

- subscriptions require an explicit `_select`
- `subscription/Trading` with no selection returns a GraphQL application error instead of a useful stream
- `subscription/EVM` is a better validation target because it produces frequent traffic
- avoid leading with `limit` in subscription selections; use a direct event shape first

Status:

- **validated successfully**

### Not Yet Validated Successfully

#### GoldRush MCP

Skill:

- [skills/goldrush-mcp-skill/SKILL.md](../skills/goldrush-mcp-skill/SKILL.md)

Observed behavior:

- stdio MCP server startup and tool help succeeded
- auth injection worked
- tool surface is usable
- MCP resource subscription using `status://all-chains` failed
- daemon status reported: `MCP server does not support resources.subscribe`

Current assessment:

- GoldRush MCP is usable as a tool-oriented MCP integration
- it is not currently a validated `uxc subscribe` provider because the live server does not expose `resources.subscribe`

Status:

- **not yet validated for subscribe**

#### Matrix Client-Server API

Skill:

- [skills/matrix-openapi-skill/SKILL.md](../skills/matrix-openapi-skill/SKILL.md)

Observed behavior:

- OAuth-backed Matrix auth worked against `matrix.org`
- poll subscribe using `get:/sync` succeeded with `since -> next_batch` cursor progression
- sparse `/sync` responses that omit the room timeline path no longer fail when
  `missing_extract_items_pointer_as_empty=true`
- direct `uxc subscribe start https://matrix.org/_matrix/client/v3 ...` works for room-scoped polling

Current limitation:

- linked `matrix-openapi-cli subscribe ...` still does not work because link-generated `--schema-url`
  flags are rejected by `subscribe`

Current assessment:

- Matrix is now a validated poll-subscribe provider when invoked directly through `uxc subscribe start`
- `#283` fixed the sparse `/sync` response handling gap that previously stopped the job after the first poll cycle

Status:

- **validated for direct poll subscribe**

Related work:

- [#274](https://github.com/holon-run/uxc/issues/274)
- [#283](https://github.com/holon-run/uxc/issues/283)

### Exploratory IM Event Intake

These providers are relevant to IM-style inbound events, but not all of them are validated as full message-ingest providers yet.

#### Discord Gateway

Skill:

- [skills/discord-openapi-skill/SKILL.md](../skills/discord-openapi-skill/SKILL.md)

Validation result:

- Discord bot auth worked against the REST API
- `discord-gateway` subscribe bootstrapped successfully through `GET /gateway/bot`
- live Gateway sessions reached `READY`
- `GUILD_CREATE` state sync arrived through the subscribe sink
- a real posted channel message produced `MESSAGE_CREATE`
- heartbeat scheduling, `IDENTIFY`, sequence tracking, and reconnect handling were exercised in live runs

Recommended runtime note:

- `4609` (`GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES`) is a safe default intent bitset
- add `32768` (`MESSAGE_CONTENT`) only when the bot has that privileged intent enabled

Status:

- **validated successfully**

#### Slack Socket Mode

Skill:

- [skills/slack-openapi-skill/SKILL.md](../skills/slack-openapi-skill/SKILL.md)

Observed behavior:

- Slack Web API auth and request/response calls are working
- the built-in `slack_socket_mode` transport succeeded against the live Slack API
- `uxc subscribe start https://slack.com/api --transport slack-socket-mode --auth slack-app ...` opened a temporary WebSocket URL automatically
- the initial Slack Socket Mode `hello` payload was received through the built-in transport
- a real Slack message event was delivered as an `events_api` envelope while the subscription job was running
- the sink recorded the message payload and `ack_sent=true`
- automatic ack behavior for envelope-style frames is implemented and covered by unit tests

Current assessment:

- Slack Socket Mode now has provider-aware runtime support inside `uxc subscribe`
- live connection setup is validated
- live inbound message event delivery is validated

Status:

- **validated successfully**

#### Feishu / Lark IM

Skill:

- [skills/feishu-openapi-skill/SKILL.md](../skills/feishu-openapi-skill/SKILL.md)

Validation result:

- Feishu `app_id + app_secret` bootstrap through `uxc auth bootstrap` succeeded
- live request/response IM calls succeeded after bootstrap-managed tenant token refresh
- built-in `feishu_long_connection` transport succeeded against the live Feishu long-connection bootstrap endpoint
- sink emitted `open` with the temporary Feishu WebSocket URL
- real `im.message.receive_v1` events were delivered through the subscription job
- live text messages sent to the bot in a `p2p` chat appeared as `data` events in the sink

Observed runtime notes:

- the transport opens a fresh temporary WebSocket URL from `/callback/ws/endpoint` on each connect attempt
- event frames arrive as Feishu protobuf binary frames, not plain text JSON WebSocket messages
- the runtime sends binary event acknowledgements and periodic ping control frames
- validated live payloads included:
  - `header.event_type = "im.message.receive_v1"`
  - `event.message.chat_type = "p2p"`
  - `event.message.message_type = "text"`
  - `event.message.content`

Status:

- **validated successfully**

#### DingTalk Messaging

Skill:

- [skills/dingtalk-openapi-skill/SKILL.md](../skills/dingtalk-openapi-skill/SKILL.md)

Current assessment:

- current skill scope is request/response only
- Stream Mode / inbound event handling has not yet been validated through `uxc subscribe`

Status:

- **not yet validated for subscribe**

## Interpretation

Current real-world status is uneven by provider:

- Binance proves the raw WebSocket runtime works against a live public market stream
- OKX proves explicit subscribe frames work for custom exchange WebSocket protocols
- Telegram proves that polling subscriptions are working against a real external API
- Bitquery now proves the GraphQL WebSocket runtime works against a live provider, but provider-specific selection shape still matters
- GoldRush shows that MCP tools and MCP resource subscriptions must be treated separately
- Matrix now validates the room-scoped `/sync` polling path; remaining UX work is mainly around linked-command ergonomics
- Feishu now validates a provider-aware long-connection transport for IM message intake

## Recommended Next Validation Order

- No additional non-IM provider is currently prioritized for subscribe validation.
- Revisit GoldRush only if the upstream MCP server later exposes `resources.subscribe`.

## Skill Documentation Implications

Current documentation status after validation work:

- Telegram skill can now describe daemon-backed polling subscribe as validated behavior
- Matrix skill can now describe direct daemon-backed `/sync` polling subscribe as validated behavior
- Bitquery skill can now describe `subscription/EVM` as validated, while still warning that subscriptions need an explicit `_select` and provider-appropriate shape
- Feishu skill can now describe `feishu-long-connection` subscribe as validated behavior
- GoldRush skill should not imply that current live MCP resources are subscribable unless upstream capability changes
