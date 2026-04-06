# Skills

This directory contains the canonical `uxc` skill plus official wrapper skills
built on top of it.

Use this directory as the main entrypoint for browsing available skills.
Use [`../docs/operations/skills.md`](../docs/operations/skills.md) for publish history, validation
notes, and maintenance logs.

## Recommended Starting Points

- [`uxc`](uxc/SKILL.md)
  - Canonical reusable execution layer for schema-described remote interfaces.
- [`playwright-mcp-skill`](playwright-mcp-skill/SKILL.md)
  - Browser automation over MCP stdio through `uxc`.
- [`context7-mcp-skill`](context7-mcp-skill/SKILL.md)
  - Current library documentation and examples over MCP.
- [`deepwiki-mcp-skill`](deepwiki-mcp-skill/SKILL.md)
  - Repository documentation and codebase Q&A over MCP.

## Skill Categories

### Core and Skill Authoring

- [`uxc`](uxc/SKILL.md)
- [`uxc-skill-creator`](uxc-skill-creator/SKILL.md)

### Browser Automation and Documentation

- [`playwright-mcp-skill`](playwright-mcp-skill/SKILL.md)
- [`chrome-devtools-mcp-skill`](chrome-devtools-mcp-skill/SKILL.md)
- [`context7-mcp-skill`](context7-mcp-skill/SKILL.md)
- [`deepwiki-mcp-skill`](deepwiki-mcp-skill/SKILL.md)
- [`qmd-mcp-skill`](qmd-mcp-skill/SKILL.md)

### Workspace and Messaging

- [`notion-mcp-skill`](notion-mcp-skill/SKILL.md)
- [`notion-openapi-skill`](notion-openapi-skill/SKILL.md)
- [`linear-graphql-skill`](linear-graphql-skill/SKILL.md)
- [`slack-openapi-skill`](slack-openapi-skill/SKILL.md)
- [`discord-openapi-skill`](discord-openapi-skill/SKILL.md)
- [`feishu-openapi-skill`](feishu-openapi-skill/SKILL.md)
- [`telegram-openapi-skill`](telegram-openapi-skill/SKILL.md)
- [`matrix-openapi-skill`](matrix-openapi-skill/SKILL.md)
- [`dingtalk-openapi-skill`](dingtalk-openapi-skill/SKILL.md)
- [`line-openapi-skill`](line-openapi-skill/SKILL.md)
- [`whatsapp-openapi-skill`](whatsapp-openapi-skill/SKILL.md)
- [`x-openapi-skill`](x-openapi-skill/SKILL.md)

### Crypto and Web3

- [`alchemy-openapi-skill`](alchemy-openapi-skill/SKILL.md)
- [`binance-spot-openapi-skill`](binance-spot-openapi-skill/SKILL.md)
- [`binance-spot-websocket-skill`](binance-spot-websocket-skill/SKILL.md)
- [`binance-web3-openapi-skill`](binance-web3-openapi-skill/SKILL.md)
- [`birdeye-mcp-skill`](birdeye-mcp-skill/SKILL.md)
- [`bitget-openapi-skill`](bitget-openapi-skill/SKILL.md)
- [`bitquery-graphql-skill`](bitquery-graphql-skill/SKILL.md)
- [`blocknative-openapi-skill`](blocknative-openapi-skill/SKILL.md)
- [`blockscout-openapi-skill`](blockscout-openapi-skill/SKILL.md)
- [`bybit-openapi-skill`](bybit-openapi-skill/SKILL.md)
- [`chainbase-openapi-skill`](chainbase-openapi-skill/SKILL.md)
- [`coinapi-openapi-skill`](coinapi-openapi-skill/SKILL.md)
- [`coinbase-openapi-skill`](coinbase-openapi-skill/SKILL.md)
- [`coingecko-openapi-skill`](coingecko-openapi-skill/SKILL.md)
- [`coinmarketcap-mcp-skill`](coinmarketcap-mcp-skill/SKILL.md)
- [`crypto-com-mcp-skill`](crypto-com-mcp-skill/SKILL.md)
- [`defillama-openapi-skill`](defillama-openapi-skill/SKILL.md)
- [`defillama-prices-openapi-skill`](defillama-prices-openapi-skill/SKILL.md)
- [`defillama-pro-openapi-skill`](defillama-pro-openapi-skill/SKILL.md)
- [`defillama-yields-openapi-skill`](defillama-yields-openapi-skill/SKILL.md)
- [`dexscreener-openapi-skill`](dexscreener-openapi-skill/SKILL.md)
- [`dune-mcp-skill`](dune-mcp-skill/SKILL.md)
- [`ethereum-jsonrpc-skill`](ethereum-jsonrpc-skill/SKILL.md)
- [`etherscan-mcp-skill`](etherscan-mcp-skill/SKILL.md)
- [`gate-mcp-skill`](gate-mcp-skill/SKILL.md)
- [`goldrush-mcp-skill`](goldrush-mcp-skill/SKILL.md)
- [`helius-openapi-skill`](helius-openapi-skill/SKILL.md)
- [`hive-mcp-skill`](hive-mcp-skill/SKILL.md)
- [`kraken-openapi-skill`](kraken-openapi-skill/SKILL.md)
- [`kucoin-openapi-skill`](kucoin-openapi-skill/SKILL.md)
- [`lifi-mcp-skill`](lifi-mcp-skill/SKILL.md)
- [`mempool-space-openapi-skill`](mempool-space-openapi-skill/SKILL.md)
- [`mexc-openapi-skill`](mexc-openapi-skill/SKILL.md)
- [`moralis-openapi-skill`](moralis-openapi-skill/SKILL.md)
- [`near-jsonrpc-skill`](near-jsonrpc-skill/SKILL.md)
- [`nodit-openapi-skill`](nodit-openapi-skill/SKILL.md)
- [`okx-exchange-websocket-skill`](okx-exchange-websocket-skill/SKILL.md)
- [`okx-mcp-skill`](okx-mcp-skill/SKILL.md)
- [`sui-jsonrpc-skill`](sui-jsonrpc-skill/SKILL.md)
- [`thegraph-mcp-skill`](thegraph-mcp-skill/SKILL.md)
- [`thegraph-token-mcp-skill`](thegraph-token-mcp-skill/SKILL.md)
- [`upbit-openapi-skill`](upbit-openapi-skill/SKILL.md)

<!-- INDEX:START -->

- [alchemy-openapi-skill](./alchemy-openapi-skill/)
  Operate Alchemy Prices API reads through UXC with a curated OpenAPI schema, path-templated API-key auth, and read-first guardrails.

- [binance-spot-openapi-skill](./binance-spot-openapi-skill/)
  Operate Binance Spot market, account, and order APIs through UXC with a curated OpenAPI schema, Binance query signing, and separate mainnet/testnet link flows.

- [binance-spot-websocket-skill](./binance-spot-websocket-skill/)
  Subscribe to Binance Spot public market streams through UXC raw WebSocket support for trades, book ticker, depth, and ticker events with stream-specific guardrails.

- [binance-web3-openapi-skill](./binance-web3-openapi-skill/)
  Operate Binance Web3 public market and research APIs through UXC with a curated OpenAPI schema. Use when tasks need token search, token metadata/market snapshots, address holdings, rankings, token audit, or smart money signals on Binance Web3.

- [birdeye-mcp-skill](./birdeye-mcp-skill/)
  Use Birdeye MCP through UXC for token market data, trending and discovery workflows, price monitoring, and DEX-related reads with help-first live tool discovery and API-key auth.

- [bitget-openapi-skill](./bitget-openapi-skill/)
  Operate Bitget public exchange market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.

- [bitquery-graphql-skill](./bitquery-graphql-skill/)
  Use Bitquery GraphQL through UXC for onchain trades, transfers, token holder analysis, balances, and market structure queries across supported networks, with OAuth client_credentials authentication and query-first execution.

- [blocknative-openapi-skill](./blocknative-openapi-skill/)
  Operate Blocknative gas intelligence APIs through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.

- [blockscout-openapi-skill](./blockscout-openapi-skill/)
  Operate Blockscout explorer reads through UXC with a curated OpenAPI schema, instance-specific host selection, and read-first guardrails.

- [bybit-openapi-skill](./bybit-openapi-skill/)
  Operate Bybit V5 public market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.

- [chainbase-openapi-skill](./chainbase-openapi-skill/)
  Operate Chainbase indexed wallet and token reads through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.

- [chrome-devtools-mcp-skill](./chrome-devtools-mcp-skill/)
  Use Chrome DevTools MCP through UXC over local stdio for page navigation, DOM/a11y snapshots, network inspection, console inspection, and performance tooling, with a live-browser autoConnect default and optional browserUrl or isolated fallback modes.

- [coinapi-openapi-skill](./coinapi-openapi-skill/)
  Operate CoinAPI market data reads through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.

- [coinbase-openapi-skill](./coinbase-openapi-skill/)
  Operate Coinbase Advanced Trade REST APIs through UXC with a curated OpenAPI schema, products-first discovery, and explicit JWT bearer auth guidance.

- [coingecko-openapi-skill](./coingecko-openapi-skill/)
  Operate CoinGecko and GeckoTerminal market data APIs through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.

- [coinmarketcap-mcp-skill](./coinmarketcap-mcp-skill/)
  Use CoinMarketCap MCP through UXC for crypto market quotes, technical analysis, on-chain metrics, global market overview, narratives, macro events, news, and semantic search with help-first schema inspection and API-key auth.

- [context7-mcp-skill](./context7-mcp-skill/)
  Query up-to-date library documentation and code examples using Context7 MCP. Use when you need current, version-specific documentation for npm packages, Python libraries, or other programming languages.

- [crypto-com-mcp-skill](./crypto-com-mcp-skill/)
  Use Crypto.com MCP through UXC for exchange market data workflows with help-first discovery and read-only guardrails.

- [deepwiki-mcp-skill](./deepwiki-mcp-skill/)
  Ask questions and read documentation about any GitHub repository using DeepWiki MCP. Use when you need to understand a codebase, find specific APIs, or get context about a repository.

- [defillama-openapi-skill](./defillama-openapi-skill/)
  Operate DefiLlama public analytics APIs through UXC with a curated OpenAPI schema and read-first guardrails.

- [defillama-prices-openapi-skill](./defillama-prices-openapi-skill/)
  Operate DefiLlama public price APIs through UXC with a curated OpenAPI schema and read-first guardrails.

- [defillama-pro-openapi-skill](./defillama-pro-openapi-skill/)
  Operate DefiLlama Pro analytics APIs through UXC with a curated OpenAPI schema, path-templated API-key auth, and read-first guardrails.

- [defillama-yields-openapi-skill](./defillama-yields-openapi-skill/)
  Operate DefiLlama public yield APIs through UXC with a curated OpenAPI schema and read-first guardrails.

- [dexscreener-openapi-skill](./dexscreener-openapi-skill/)
  Operate DexScreener public market data APIs through UXC with a curated OpenAPI schema, no-auth setup, and read-first guardrails.

- [dingtalk-openapi-skill](./dingtalk-openapi-skill/)
  Operate DingTalk messaging APIs through UXC with a curated OpenAPI schema, app-token bearer auth, and robot/service-group guardrails.

- [discord-openapi-skill](./discord-openapi-skill/)
  Operate Discord HTTP API through UXC with Discord OpenAPI schema. Bot token recommended for full API access including messages and server management. OAuth2 user authentication available for limited profile operations only.

- [dune-mcp-skill](./dune-mcp-skill/)
  Use Dune MCP through UXC for blockchain table discovery, SQL query creation/execution, execution result retrieval, and visualization with help-first schema inspection, explicit auth binding, and guarded credit-consuming operations.

- [ethereum-jsonrpc-skill](./ethereum-jsonrpc-skill/)
  Operate Ethereum execution JSON-RPC through UXC with the official execution OpenRPC schema, public EVM read methods, and eth_subscribe pubsub guardrails.

- [etherscan-mcp-skill](./etherscan-mcp-skill/)
  Use Etherscan MCP through UXC for address balance checks, token holder analysis, transaction inspection, and contract lookup tasks. Use when tasks need Etherscan MCP tools for onchain investigation with help-first schema inspection, bearer-key auth, and tier-aware read-first handling.

- [feishu-openapi-skill](./feishu-openapi-skill/)
  Operate Feishu or Lark IM APIs through UXC with a curated OpenAPI schema, tenant-token bearer auth, and chat/message guardrails.

- [gate-mcp-skill](./gate-mcp-skill/)
  Use Gate MCP through UXC for public spot and futures market data workflows with a fixed streamable-http endpoint and read-first guardrails.

- [goldrush-mcp-skill](./goldrush-mcp-skill/)
  Use GoldRush MCP through UXC for multichain wallet balances, transfers, portfolio history, NFT ownership, token approvals, prices, and chain metadata via stdio MCP with injected API-key auth.

- [helius-openapi-skill](./helius-openapi-skill/)
  Operate Helius Wallet API reads through UXC with a curated OpenAPI schema, API-key auth, and read-first guardrails.

- [hive-mcp-skill](./hive-mcp-skill/)
  Use Hive Intelligence MCP through UXC for broad crypto market, onchain, portfolio, and risk workflows with help-first discovery and convenience-layer guardrails.

- [kraken-openapi-skill](./kraken-openapi-skill/)
  Operate Kraken public market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.

- [kucoin-openapi-skill](./kucoin-openapi-skill/)
  Operate KuCoin public exchange market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.

- [lifi-mcp-skill](./lifi-mcp-skill/)
  Use the LI.FI MCP server through UXC for cross-chain route discovery, bridge/DEX availability checks, token and chain lookup, gas/balance/allowance checks, quote generation, and transfer status tracking. Use when tasks involve planning or monitoring cross-chain swaps and bridges without signing or broadcasting transactions.

- [line-openapi-skill](./line-openapi-skill/)
  Operate LINE Messaging API through UXC with a curated OpenAPI schema, bearer-token auth, and messaging-core guardrails.

- [linear-graphql-skill](./linear-graphql-skill/)
  Operate Linear workspace issues, projects, and teams through Linear GraphQL API using UXC. Use when tasks require querying or creating issues, managing projects, or interacting with Linear workflow. Supports both Personal API Key and OAuth authentication.

- [matrix-openapi-skill](./matrix-openapi-skill/)
  Operate Matrix Client-Server API through UXC with a curated OpenAPI schema, bearer-token auth, and homeserver-aware messaging guardrails.

- [mempool-space-openapi-skill](./mempool-space-openapi-skill/)
  Operate mempool.space public Bitcoin and Lightning explorer APIs through UXC with a curated OpenAPI schema, no-auth setup, and read-first guardrails.

- [mexc-openapi-skill](./mexc-openapi-skill/)
  Operate MEXC Spot REST APIs through UXC with a curated OpenAPI schema, HMAC query signing, and separate public/signed workflow guardrails.

- [moralis-openapi-skill](./moralis-openapi-skill/)
  Operate Moralis EVM wallet and token reads through UXC with a curated OpenAPI schema, API-key auth, and wallet-intelligence guardrails.

- [near-jsonrpc-skill](./near-jsonrpc-skill/)
  Operate NEAR JSON-RPC reads through UXC with a public provider default, provider-override guidance, and read-only guardrails.

- [nodit-openapi-skill](./nodit-openapi-skill/)
  Operate Nodit Web3 Data API reads through UXC with a curated OpenAPI schema, API-key auth, and overlap-aware guardrails.

- [notion-mcp-skill](./notion-mcp-skill/)
  Operate Notion workspace content through Notion MCP using the UXC CLI, including search, fetch, users/teams lookup, page/database creation and updates, and comments. Use when tasks require calling Notion tools over MCP with OAuth (authorization_code + PKCE), especially when safe write controls and JSON-envelope parsing are required.

- [notion-openapi-skill](./notion-openapi-skill/)
  Operate Notion Public API through UXC with a curated OpenAPI schema for search, block traversal, page reads, content writes, and data source/database inspection. Use when tasks need recursive reads or structured writes that Notion MCP does not expose directly.

- [okx-exchange-websocket-skill](./okx-exchange-websocket-skill/)
  Subscribe to OKX public exchange WebSocket channels through UXC raw WebSocket mode for ticker, trade, book, and candle events with explicit subscribe frames.

- [okx-mcp-skill](./okx-mcp-skill/)
  Use OKX OnchainOS MCP through UXC for token discovery, market data, wallet balance, and swap execution planning. Use when tasks need OKX MCP tools such as token search/ranking/holder, price/trades/candlesticks/index, balance queries, and DEX quote/swap flows with help-first schema inspection and safe auth handling.

- [playwright-mcp-skill](./playwright-mcp-skill/)
  Run browser automation through @playwright/mcp over UXC stdio MCP, with daemon-friendly session reuse and safe action guardrails. Use when tasks need deterministic page navigation, DOM snapshots, and scripted browser interaction from CLI.

- [qmd-mcp-skill](./qmd-mcp-skill/)
  Use a local QMD knowledge base through UXC over MCP stdio, with daemon-backed session reuse and typed retrieval flows that avoid repeated model warmup and unnecessary query-expansion latency.

- [slack-openapi-skill](./slack-openapi-skill/)
  Operate Slack Web API through UXC with a curated OpenAPI schema, bearer-token auth, and messaging-core guardrails.

- [sui-jsonrpc-skill](./sui-jsonrpc-skill/)
  Operate Sui public JSON-RPC through UXC with OpenRPC-driven discovery, mainnet fullnode defaults, and read-only query plus pubsub subscription guardrails.

- [telegram-openapi-skill](./telegram-openapi-skill/)
  Operate Telegram Bot API through UXC with a curated OpenAPI schema, bot-token path auth, polling-based reads, and webhook management guardrails.

- [thegraph-mcp-skill](./thegraph-mcp-skill/)
  Use The Graph Subgraph MCP through UXC via native SSE with a fixed linked command for subgraph discovery, schema retrieval, deployment selection, and GraphQL query execution with help-first inspection and explicit auth handling.

- [thegraph-token-mcp-skill](./thegraph-token-mcp-skill/)
  Use The Graph Token API MCP through UXC for token metadata, wallet balances, transfers, holders, pools, and market data with help-first inspection and Token API specific JWT bearer auth binding.

- [upbit-openapi-skill](./upbit-openapi-skill/)
  Operate Upbit public exchange market APIs through UXC with a curated OpenAPI schema, market-first discovery, and explicit private-auth boundary notes.

- [uxc](./uxc/)
  Discover and call remote schema-exposed interfaces with UXC. Use when an agent or skill needs to list operations, inspect operation schemas, and execute OpenAPI, GraphQL, gRPC, MCP, or JSON-RPC calls via one CLI contract.

- [uxc-skill-creator](./uxc-skill-creator/)
  Create wrapper skills that call remote tools through UXC. Use when defining a new provider skill and you need reusable templates, validation rules, and anti-pattern guidance based on proven UXC skill practices.

- [whatsapp-openapi-skill](./whatsapp-openapi-skill/)
  Operate WhatsApp Business Platform Cloud API through UXC with a curated OpenAPI schema, bearer-token auth, and message/profile guardrails.

- [x-openapi-skill](./x-openapi-skill/)
  Operate X API v2 through UXC with the official OpenAPI schema, OAuth2 PKCE user-context auth, app-only bearer guidance, and read-first guardrails for timeline/bookmark/post workflows.

<!-- INDEX:END -->
