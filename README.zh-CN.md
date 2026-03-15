# UXC

**通用 X 协议命令行工具（Universal X-Protocol CLI）**

为 Agent 提供稳定的执行界面。

[English](README.md) | 简体中文

[![CI](https://github.com/holon-run/uxc/workflows/CI/badge.svg)](https://github.com/holon-run/uxc/actions)
[![Coverage](https://github.com/holon-run/uxc/workflows/Coverage/badge.svg)](https://github.com/holon-run/uxc/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)

UXC 为 Agent 提供一种稳定的方式，用来发现、认证并调用跨 OpenAPI、gRPC、GraphQL、MCP 和 JSON-RPC 的远端工具。

它不要求你为每种协议、每个 SDK 或每套本地 MCP 配置各写一层胶水代码，而是把这些远端接口统一成一个可预测的命令契约，支持 help-first discovery、结构化执行和确定性的 JSON 输出。

## 为什么要做 UXC

Agent 的工具调用通常会在同几个地方变得很痛苦：

- 认证信息散落在 prompt、脚本和本地配置里
- 每种协议都有不同的调用方式
- 本地 MCP server 名称和配置不能跨机器复用
- 大体量工具清单或 schema 被直接塞进上下文
- 为每个 endpoint 单独写的 wrapper 很快就会和上游接口漂移

UXC 的目标，是让远端能力对 Agent 和自动化任务来说，都表现成一个稳定的执行界面。

## UXC 做什么

- 按需从 endpoint 发现操作
- 在执行前查看输入和输出形状
- 用结构化参数执行操作
- 默认返回确定性的 JSON envelope
- 复用 auth 绑定、signer 配置和 link 快捷命令

如果一个目标能描述自己，UXC 通常就能调用它。

## 为什么它适合 Agent 和 Skill

- 渐进式发现可以保持上下文更小：先 `<host> -h`，再 `<host> <operation_id> -h`，最后执行
- URL-first 的方式不依赖机器相关的 MCP alias 或本地 wrapper 名称
- Auth binding 把凭证匹配规则从 prompt 中剥离出来
- `uxc link` 可以把远端 endpoint 固化成稳定的本地命令
- 同一套命令契约可以被多个 skill 和 workflow 复用

## 核心能力

- URL-first 使用方式：直接调用 endpoint，不需要先定义 server alias
- 多协议检测与 adapter 路由
- 基于 schema 的操作发现（`<host> -h`, `<host> <operation_id> -h`）
- 结构化调用（位置 JSON、key=value）
- 面向自动化与 agent 的确定性 JSON envelope
- 可复用凭证、binding 和 signer profile 的认证模型
- 面向短期 bearer token 的 app 凭证 bootstrap（例如 Feishu/Lark、DingTalk）
- 通过 `uxc link` 提供 host 快捷命令
- 通过 `uxc link --schema-url` 为 OpenAPI link 持久化默认 schema 地址
- 通过 `uxc subscribe` 提供 daemon 驱动的后台订阅能力
- 面向 Slack Socket Mode、Discord Gateway、Feishu 长连接的 provider-aware 事件接入
- 通过 `--inject-env NAME={{secret}}` 为 stdio 子进程注入认证环境变量

支持协议：

- OpenAPI / Swagger
- gRPC（server reflection）
- GraphQL（introspection）
- MCP（HTTP 和 stdio）
- JSON-RPC（基于 OpenRPC 发现）

## 架构快照

UXC 将协议差异统一到一个执行契约之下：

![UXC 架构快照](docs/images/uxc-architecture-full.png)

这个设计让 discovery、auth 和 execution 的使用方式保持稳定，同时允许各协议内部实现独立演进。

## 目标使用场景

- 需要确定性远端工具调用的 AI agent 与 skill
- 不想做 SDK 初始化、希望基于 schema 直接调用的 CI/CD 与自动化任务
- 用统一命令契约做跨协议集成测试
- 需要 JSON envelope 与可预测错误模型的受控运行环境

## 非目标

UXC 不是：

- 代码生成器
- SDK 框架
- API 网关或反向代理

UXC 的定位是：为可自描述的远端能力提供稳定的执行界面。

## 安装

### Homebrew（macOS/Linux）

```bash
brew tap holon-run/homebrew-tap
brew install uxc
```

### 安装脚本（macOS/Linux）

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash
```

运行前可先审阅脚本：

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh -o install-uxc.sh
less install-uxc.sh
bash install-uxc.sh
```

安装指定版本：

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash -s -- -v v0.12.0
```

Windows 说明：不再支持原生 Windows，请通过 WSL 运行 UXC。

### Cargo

```bash
cargo install uxc
```

### 从源码安装

```bash
git clone https://github.com/holon-run/uxc.git
cd uxc
cargo install --path .
```

## 快速开始（3 分钟）

多数 HTTP 示例省略协议前缀。
对于公网主机，UXC 在省略时默认推断为 `https://`。

1. 发现操作：

```bash
uxc petstore3.swagger.io/api/v3 -h
```

2. 查看操作 schema：

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
```

3. 使用结构化输入执行调用：

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

仅使用以下 endpoint 形式：
- `uxc <host> -h`
- `uxc <host> <operation_id> -h`
- `uxc <host> <operation_id> key=value` 或 `uxc <host> <operation_id> '{...}'`

## 协议示例（每种一个）

`operation_id` 约定：

- OpenAPI：`method:/path`（例如 `get:/users/{id}`）
- gRPC：`Service/Method`
- GraphQL：`query/viewer`、`mutation/createUser`
- MCP：工具名（例如 `ask_question`）
- JSON-RPC：方法名（例如 `eth_getBalance`）

### OpenAPI

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

对于 schema 与服务地址分离的服务，可覆盖 schema 来源：

```bash
uxc api.github.com -h \
  --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

### gRPC

```bash
uxc grpcb.in:9000 -h
uxc grpcb.in:9000 addsvc.Add/Sum a=1 b=2
```

说明：gRPC unary 运行时调用依赖 `grpcurl` 在 `PATH` 中可用。

### GraphQL

```bash
uxc countries.trevorblades.com -h
uxc countries.trevorblades.com query/country code=US
# 非字符串对象参数推荐使用位置 JSON
uxc api.linear.app/graphql mutation/issueCreate '{"input":{"teamId":"TEAM_ID","title":"Test"}}'
# 可选：通过保留参数 _select 指定 GraphQL 返回字段
uxc api.linear.app/graphql query/issues '{"first":5,"_select":"nodes { identifier title url state { name } }"}'
```

### MCP

```bash
uxc mcp.deepwiki.com/mcp -h
uxc mcp.deepwiki.com/mcp ask_question repoName=holon-run/uxc question='What does this project do?'
```

### MCP（stdio）

UXC 也可以通过 stdio 调用本地进程启动的 MCP server。
对 stdio endpoint 来说，“URL”就是一个带引号的命令行。

Playwright MCP（stdio）示例：

```bash
# 一次性发现
uxc "npx -y @playwright/mcp@latest --headless --isolated" -h

# 为重复使用创建稳定命令名（推荐）
uxc link playwright-mcp-cli "npx -y @playwright/mcp@latest --headless --isolated"
playwright-mcp-cli -h

# 调用前先查看操作 schema
playwright-mcp-cli browser_navigate -h

# 使用 key=value 调用
playwright-mcp-cli browser_navigate url=https://example.com
playwright-mcp-cli browser_snapshot
```

### JSON-RPC

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

## Skills

UXC 提供一个标准 skill（`uxc`）和若干官方场景化 wrapper skill。
建议把 `uxc` 作为共享执行层，再按场景补充 wrapper。

### 从这里开始

- [`uxc`](skills/uxc/SKILL.md)：标准的 schema 发现与多协议执行层
- [`playwright-mcp-skill`](skills/playwright-mcp-skill/SKILL.md)：通过 `uxc` 在 MCP stdio 上做浏览器自动化
- [`context7-mcp-skill`](skills/context7-mcp-skill/SKILL.md)：查询最新库文档与示例

### 核心能力

- [`uxc`](skills/uxc/SKILL.md)：标准的 schema 发现与多协议执行层
- [`uxc-skill-creator`](skills/uxc-skill-creator/SKILL.md)：创建 UXC skill 的模板与工作流指南

### 浏览器自动化

- [`playwright-mcp-skill`](skills/playwright-mcp-skill/SKILL.md)：通过 `uxc` 在 MCP stdio 上运行 `@playwright/mcp`

### 知识与文档

- [`context7-mcp-skill`](skills/context7-mcp-skill/SKILL.md)：查询最新库文档与示例
- [`deepwiki-mcp-skill`](skills/deepwiki-mcp-skill/SKILL.md)：查询仓库文档并回答代码库问题

### 协作与工作流

- [`notion-mcp-skill`](skills/notion-mcp-skill/SKILL.md)：提供带 OAuth 说明的 Notion MCP 流程指引
- [`linear-graphql-skill`](skills/linear-graphql-skill/SKILL.md)：通过 GraphQL 操作 Linear 的 issue、project 和 team
- [`slack-openapi-skill`](skills/slack-openapi-skill/SKILL.md)：通过 UXC 操作 Slack Web API，并通过 `uxc subscribe` 接收 Socket Mode 事件
- [`discord-openapi-skill`](skills/discord-openapi-skill/SKILL.md)：通过 UXC + OpenAPI schema mapping 操作 Discord REST API，并通过 `uxc subscribe` 接收 Gateway 事件
- [`feishu-openapi-skill`](skills/feishu-openapi-skill/SKILL.md)：通过 UXC 操作飞书/Lark IM API，并使用自动 bootstrap 鉴权和长连接事件接收
- [`telegram-openapi-skill`](skills/telegram-openapi-skill/SKILL.md)：通过 UXC 操作 Telegram Bot API，并通过轮询式 `uxc subscribe` 接收更新
- [`matrix-openapi-skill`](skills/matrix-openapi-skill/SKILL.md)：通过 UXC 操作 Matrix Client-Server API，并通过 `/sync` 轮询跟踪房间时间线

### Crypto 与链上数据

- [`bitquery-graphql-skill`](skills/bitquery-graphql-skill/SKILL.md)：通过 Bitquery GraphQL 查询链上交易、转账、持币人、余额与市场结构，使用 OAuth client credentials 鉴权
- [`binance-web3-openapi-skill`](skills/binance-web3-openapi-skill/SKILL.md)：通过 OpenAPI 查询 Binance Web3 的代币发现、榜单、聪明钱、审计与地址持仓
- [`binance-spot-openapi-skill`](skills/binance-spot-openapi-skill/SKILL.md)：通过 OpenAPI 操作 Binance Spot 公共市场数据与签名账户/订单接口
- [`binance-spot-websocket-skill`](skills/binance-spot-websocket-skill/SKILL.md)：通过原始 WebSocket 订阅 Binance Spot 的成交、ticker、深度与最优买卖盘流
- [`okx-exchange-websocket-skill`](skills/okx-exchange-websocket-skill/SKILL.md)：通过原始 WebSocket 订阅 OKX 公共交易所的 ticker、成交、盘口与 K 线频道
- [`okx-mcp-skill`](skills/okx-mcp-skill/SKILL.md)：通过 OKX MCP 执行 token、市场、钱包、换汇等流程
- [`dune-mcp-skill`](skills/dune-mcp-skill/SKILL.md)：通过 Dune MCP 发现链上表、执行 SQL、获取结果并生成图表
- [`thegraph-mcp-skill`](skills/thegraph-mcp-skill/SKILL.md)：通过 The Graph Subgraph MCP bridge 发现 subgraph、查看 schema 并执行 GraphQL
- [`thegraph-token-mcp-skill`](skills/thegraph-token-mcp-skill/SKILL.md)：通过 The Graph Token API MCP 查询 token、钱包、转账、持仓人与市场数据
- [`etherscan-mcp-skill`](skills/etherscan-mcp-skill/SKILL.md)：通过 Etherscan MCP 调查地址、持币人和合约信息

### Skill 安装方式

使用 `npx skills` 从本仓库安装：

```bash
# 先安装共享执行层
npx -y skills@latest add holon-run/uxc --skill uxc --agent codex -y

# 安装浏览器和知识类 skill
npx -y skills@latest add holon-run/uxc --skill playwright-mcp-skill --skill context7-mcp-skill --skill deepwiki-mcp-skill --agent codex -y

# 安装协作类 skill
npx -y skills@latest add holon-run/uxc --skill notion-mcp-skill --skill linear-graphql-skill --skill slack-openapi-skill --skill discord-openapi-skill --skill feishu-openapi-skill --skill telegram-openapi-skill --skill matrix-openapi-skill --agent codex -y

# 安装 Crypto 与链上类 skill
npx -y skills@latest add holon-run/uxc --skill bitquery-graphql-skill --skill binance-web3-openapi-skill --skill binance-spot-openapi-skill --skill binance-spot-websocket-skill --skill okx-exchange-websocket-skill --skill okx-mcp-skill --skill dune-mcp-skill --skill thegraph-mcp-skill --skill thegraph-token-mcp-skill --skill etherscan-mcp-skill --agent codex -y
```

使用 ClawHub 安装已发布的 skill：

```bash
# 先安装共享执行层
clawhub --workdir ~/.openclaw --dir skills install uxc

# 安装浏览器和知识类 skill
clawhub --workdir ~/.openclaw --dir skills install playwright-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install context7-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install deepwiki-mcp-skill

# 安装协作类 skill
clawhub --workdir ~/.openclaw --dir skills install notion-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install linear-graphql-skill
clawhub --workdir ~/.openclaw --dir skills install slack-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install discord-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install feishu-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install telegram-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install matrix-openapi-skill

# 安装 Crypto 与链上类 skill
clawhub --workdir ~/.openclaw --dir skills install binance-web3-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install binance-spot-openapi-skill
clawhub --workdir ~/.openclaw --dir skills install binance-spot-websocket-skill
clawhub --workdir ~/.openclaw --dir skills install okx-exchange-websocket-skill
clawhub --workdir ~/.openclaw --dir skills install okx-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install dune-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install thegraph-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install thegraph-token-mcp-skill
clawhub --workdir ~/.openclaw --dir skills install etherscan-mcp-skill
```

安装和维护规则见 [`docs/skills.md`](docs/skills.md)。

## 输出与帮助约定

UXC 默认输出 JSON。
需要人类可读输出时，使用 `--text`（或 `--format text`）。

示例：

```bash
uxc
uxc help
uxc <host> -h
uxc <host> <operation_id> -h
uxc --text help
```

说明：在 endpoint 路由中，`help` 会被当作操作名，而不是帮助别名。

成功 envelope 形状：

```json
{
  "ok": true,
  "kind": "call_result",
  "protocol": "openapi",
  "endpoint": "https://petstore3.swagger.io/api/v3",
  "operation": "get:/pet/{petId}",
  "data": {},
  "meta": {
    "version": "v1",
    "duration_ms": 128
  }
}
```

对 MCP `tools/call`，`data` 可能包含 `content`、可选 `structuredContent` 和可选 `isError`。

失败 envelope 形状：

```json
{
  "ok": false,
  "error": {
    "code": "INVALID_ARGUMENT",
    "message": "Field 'id' must be an integer"
  },
  "meta": {
    "version": "v1"
  }
}
```

## 认证（凭证 + 绑定）

UXC 认证由两类资源组成：

- Credential：密钥材料和认证类型
- Binding：endpoint 匹配规则，用于选择 credential

示例：

```bash
uxc auth credential set deepwiki --auth-type bearer --secret-env DEEPWIKI_TOKEN
uxc auth credential set deepwiki --secret-op op://Engineering/deepwiki/token
uxc auth binding add --id deepwiki-mcp --host mcp.deepwiki.com --path-prefix /mcp --scheme https --credential deepwiki --priority 100

# api_key 支持可配置的 header 名称和模板
uxc auth credential set okx --auth-type api_key --secret-env OKX_ACCESS_KEY --api-key-header OK-ACCESS-KEY
uxc auth credential set okx-advanced --auth-type api_key --header "OK-ACCESS-KEY={{secret}}" --header "OK-ACCESS-PASSPHRASE={{env:OKX_PASSPHRASE}}"
```

对 `--secret-op`，secret 在请求运行时通过 daemon 解析。
请确保 daemon 有可用的 1Password 认证上下文（例如 `OP_SERVICE_ACCOUNT_TOKEN`），并在环境变量变化后重启 daemon。

MCP HTTP 也支持 OAuth（device code、client credentials、authorization code + PKCE）。
完整流程见 [`docs/oauth-mcp-http.md`](docs/oauth-mcp-http.md)。

## 文档导航

- 扩展 quickstart 与协议 walkthrough：[`docs/quickstart.md`](docs/quickstart.md)
- 无需 key 的公开 endpoint（协议检查）：[`docs/public-endpoints.md`](docs/public-endpoints.md)
- `RUST_LOG` 日志与排障：[`docs/logging.md`](docs/logging.md)
- Auth secret 来源（`literal/env/op`）：[`docs/auth-secret-sources.md`](docs/auth-secret-sources.md)
- 通过服务管理器运行 daemon（`systemd`/`launchd`）：[`docs/daemon-service.md`](docs/daemon-service.md)
- OpenAPI schema mapping 与 `--schema-url`：[`docs/schema-mapping.md`](docs/schema-mapping.md)
- Skills 概览与安装/维护：[`docs/skills.md`](docs/skills.md)
- 发布流程：[`docs/release.md`](docs/release.md)

## 贡献

欢迎贡献。

- 开发流程与质量标准：[`CONTRIBUTING.md`](CONTRIBUTING.md)
- CI 与发布流程：[GitHub Actions](https://github.com/holon-run/uxc/actions)

## 许可证

MIT License，见 [`LICENSE`](LICENSE)。
