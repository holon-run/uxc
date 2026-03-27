# UXC

**统一的跨协议工具调用 CLI**

[English](README.md) | 简体中文

[文档站点](https://uxc.holon.run) | [Skills 目录](https://uxc.holon.run/skills/)

[![CI](https://github.com/holon-run/uxc/workflows/CI/badge.svg)](https://github.com/holon-run/uxc/actions)
[![Coverage](https://github.com/holon-run/uxc/workflows/Coverage/badge.svg)](https://github.com/holon-run/uxc/actions/workflows/coverage.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.83%2B-orange.svg)](https://www.rust-lang.org)

UXC 为 Agent 和自动化程序提供一致的方式，用来发现并调用
OpenAPI、MCP、GraphQL、gRPC 和 JSON-RPC 上的 API 与工具。

从 discovery 到结构化调用，UXC 试图让不同协议遵循一致的调用路径，
并在背后处理认证、输出格式和协议相关差异。

## 从这里开始

多数使用路径都遵循同一个模式：

```bash
uxc <host> -h
uxc <host> <operation_id> -h
uxc <host> <operation_id> key=value
```

示例：

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

## 为什么要做 UXC

单独访问某个远端能力通常不难，但想把不同系统里的能力稳定复用起来，
事情就会迅速变复杂。

常见痛点包括：

- 每种协议都有不同的发现和调用方式
- 认证配置散落在脚本、prompt、SDK 和本地环境里
- 执行前很难快速看清工具或 schema 细节
- 不同 provider 的输出和错误模型不一致，自动化容易变脆
- 本地 agent workflow 不希望为每个服务单独维护一层 wrapper

UXC 的目标，是把可由 schema 描述的远端能力收敛成一个可复用的 CLI
入口，供 agent、skill、脚本和本地应用共同使用。

## 它如何工作

UXC 试图把顶层交互保持得足够简单：

1. 先发现一个 host 暴露了什么能力。
2. 再查看某个具体操作的输入形状。
3. 然后用结构化参数完成调用。
4. 最后把这套调用方式复用到不同协议上。

这样一来，远端接口更像一个稳定的命令入口，而不是一堆协议各异的
request 风格。

## 为什么不是 curl、SDK 或 MCP-only 工具

### `curl`

`curl` 是 request-first 的工具。
当调用方已经知道 URL、method、headers 和 payload 形状时，它非常好用。

UXC 则更偏 discovery-first。
它帮助调用方先查看有哪些能力、理解输入形状，再用更稳定的命令契约发起调用。

### Provider SDK 或协议专用 CLI

Provider SDK 往往很强大，但每一套 SDK 都会带来自己的使用模型、认证方式
和输出形状。

UXC 的取舍，是用跨 provider、跨协议的共享接口来换取统一的使用方式。

### MCP-only tool calling

MCP 是很重要的一部分生态，但很多有价值的系统仍然通过
OpenAPI、GraphQL、gRPC 或 JSON-RPC 暴露能力。

UXC 的目标，是把 MCP 和这些相邻协议统一到一起，而不是只停留在
MCP-only workflow。

## 你能得到什么

- 通过 `<host> -h` 和 `<host> <operation_id> -h` 实现 help-first discovery
- 用 key/value 参数或位置 JSON 做结构化调用
- 默认返回稳定的 JSON 输出，文本输出为可选模式
- 可复用的认证凭证和 endpoint binding
- 面向常用 host 的快捷 link
- daemon 驱动的会话复用和后台订阅
- 用于本地集成的 TypeScript daemon client

## 协议覆盖

UXC 目前通过同一套 CLI 契约支持这些协议族：

- OpenAPI / Swagger
- MCP（HTTP 和 stdio）
- GraphQL introspection 与执行
- 基于 reflection 的 gRPC 发现与 unary 调用
- 基于 OpenRPC 风格发现的 JSON-RPC

相关运行时能力还包括：

- daemon 驱动的订阅生命周期管理
- 基于 WebSocket 的订阅流程
- 轮询式订阅
- 面向 Slack、Discord、Feishu 等系统的 provider-aware 事件接入

## 认证能力

UXC 不只是面向公开演示 endpoint，也提供了适用于真实 provider 集成的
可复用认证和 binding 原语。

支持的认证模式包括：

- bearer token
- 可配置 header 或 query 位置的 API key
- 面向签名 API 的多字段凭证
- signer 驱动的请求生成
- 面向 MCP HTTP 的 OAuth 支持
- 通过 literal、环境变量或外部 secret provider 获取 secret

主要 auth 模型是：

- credentials 存储认证材料
- bindings 负责匹配 endpoint 并选择使用哪个 credential

这样可以把 auth 配置做成可复用资产，而不是把规则和 secret 写进每条命令里。

## Skills 与集成

UXC 不只是一个 CLI 入口，这个仓库也在共享执行层之上提供了一批持续增长的
官方 skills。

代表性分类包括：

- 浏览器与本地工具：`playwright-mcp-skill`、`chrome-devtools-mcp-skill`
- 文档与研究：`context7-mcp-skill`、`deepwiki-mcp-skill`
- 协作与消息：`notion-*`、`slack-*`、`discord-*`、`telegram-*`
- Crypto 与市场数据：`dune-*`、`etherscan-*`、`thegraph-*`、`coinmarketcap-*`

建议把基础的 `uxc` skill 当作共享执行层，再按具体服务场景叠加 wrapper skill。

完整 skill 目录见 [文档站点中的 Skills 目录](https://uxc.holon.run/skills/)，
发布记录与维护日志见 [`docs/operations/skills.md`](docs/operations/skills.md)。

## 适用场景

UXC 适合：

- 需要用一套稳定方式调用多个远端系统的 agent 与 skill 作者
- 需要结构化输出和可预测失败模型的自动化脚本
- 希望使用 daemon 做会话复用，而不是解析 CLI stdout 的本地应用
- 不想让 auth 与调用方式在多 provider workflow 里不断漂移的团队

UXC 不是：

- 托管平台
- API 网关
- 对所有 provider SDK 的替代品
- 完整的 bot framework 或 workflow orchestration system

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

运行前可先审阅：

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh -o install-uxc.sh
less install-uxc.sh
bash install-uxc.sh
```

安装指定版本：

```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh | bash -s -- -v v0.12.4
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

## 快速示例

### OpenAPI

```bash
uxc petstore3.swagger.io/api/v3 -h
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

### GraphQL

```bash
uxc countries.trevorblades.com -h
uxc countries.trevorblades.com query/country code=US
```

### MCP

```bash
uxc mcp.deepwiki.com/mcp -h
uxc mcp.deepwiki.com/mcp ask_question '{"repoName":"holon-run/uxc","question":"What does this project do?"}'
```

### JSON-RPC

```bash
uxc fullnode.mainnet.sui.io -h
uxc fullnode.mainnet.sui.io sui_getLatestCheckpointSequenceNumber
```

## 输出与帮助约定

UXC 默认采用 JSON-first 输出。
需要人类可读的 CLI 输出时，可以使用 `--text` 或 `--format text`。

成功响应会返回稳定的 JSON envelope，常见字段包括：

- `ok`
- `kind`
- `protocol`
- `endpoint`
- `operation`
- `data`
- `meta`

这让 UXC 更适合被 agent、脚本和本地应用消费。

## TypeScript Daemon Client

面向本地应用集成，UXC 还提供官方的 Node/TypeScript client：

```bash
npm install @holon-run/uxc-daemon-client
```

它直接连接本地 daemon socket，返回结构化对象，而不是 CLI stdout envelope。

当需要在应用里嵌入运行时调用、daemon 状态查询，或订阅生命周期与事件流管理时，
可以优先使用它。

daemon 合约见 [站点中的 daemon API 文档](https://uxc.holon.run/daemon/api/)。

## 文档导航

- 入门： [https://uxc.holon.run/getting-started/](https://uxc.holon.run/getting-started/)
- 免 API key 的公开 endpoint： [https://uxc.holon.run/reference/public-endpoints/](https://uxc.holon.run/reference/public-endpoints/)
- auth secret 来源： [https://uxc.holon.run/auth/secret-sources/](https://uxc.holon.run/auth/secret-sources/)
- MCP HTTP OAuth： [https://uxc.holon.run/auth/oauth-mcp-http/](https://uxc.holon.run/auth/oauth-mcp-http/)
- daemon service 部署： [https://uxc.holon.run/daemon/service/](https://uxc.holon.run/daemon/service/)
- daemon API 与 TypeScript client： [https://uxc.holon.run/daemon/api/](https://uxc.holon.run/daemon/api/)
- 日志与排障： [https://uxc.holon.run/daemon/logging/](https://uxc.holon.run/daemon/logging/)
- schema mapping 与 `--schema-url`： [https://uxc.holon.run/reference/schema-mapping/](https://uxc.holon.run/reference/schema-mapping/)
- skills 目录： [https://uxc.holon.run/skills/](https://uxc.holon.run/skills/)
- skills 发布记录与维护日志：[`docs/operations/skills.md`](docs/operations/skills.md)
- 发布流程：[`docs/operations/release.md`](docs/operations/release.md)

## 贡献

欢迎贡献。

- 开发工作流：[`CONTRIBUTING.md`](CONTRIBUTING.md)
- CI 与发布流程：[GitHub Actions](https://github.com/holon-run/uxc/actions)

## 许可证

MIT License，见 [`LICENSE`](LICENSE)。
