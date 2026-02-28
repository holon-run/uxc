# CLI Dynamic Help + JSON-First Refactor Plan

Date: 2026-02-23
Status: Draft

## 背景

当前 CLI 形态主要是：

- `uxc <host> list`
- `uxc <host> call <operation> --input-json ...`
- `uxc <host> call <operation> --op-help`

已知问题：

1. 默认 `list` 输出偏文本，机器消费信息不足。
2. `operation_help` 是字符串，不是结构化数据，skill/agent 很难渐进式调用。
3. 不支持更自然的逐级探索语义（如 `uxc <host> help`、`uxc <host> <op> help`）。
4. 目前只有 `call` 稳定 JSON 包装，发现类命令没有统一 JSON 协议。

## 目标

1. 支持“动态子命令体验”：通过 `help|-h` 逐层展开 host 和 operation。
2. 全命令默认输出 JSON；文本输出需显式参数。
3. 对 AI/skill 提供稳定、可解析、可演进的发现与执行模型。
4. 允许引入破坏性变更，以简化设计并加速收敛。

## 非目标

1. 不在本阶段新增协议类型。
2. 不在本阶段引入交互式 TUI。
3. 不在本阶段改动 adapter 网络栈或缓存策略。
4. 不保证老 CLI 语法兼容。

## 新 CLI 语义（提案）

## 顶层

- `uxc -h`：静态全局帮助（clap 默认）。
- `uxc <host> help` 或 `uxc <host> -h`：host 级动态帮助。
- `uxc <host> list`：返回操作列表（JSON 默认）。
- `uxc <host> describe <operation>`：返回操作详情（JSON 默认）。
- `uxc <host> <operation> --input-json '{...}'`：直接执行（动态快捷语法）。

## 旧命令处理

- 无。旧语法在重构后直接移除。

## 输出格式参数

新增全局参数：

- `--format json|text`（默认 `json`）
- `--text`（等价于 `--format text`）

说明：

- 默认 JSON 面向 AI/skill/automation。
- 文本模式仅用于人类阅读，字段信息不减少，只改变渲染。

## JSON 协议（统一 envelope）

所有命令默认返回：

```json
{
  "ok": true,
  "kind": "operation_list",
  "protocol": "mcp",
  "endpoint": "https://mcp.deepwiki.com/mcp",
  "data": {},
  "meta": {
    "duration_ms": 31,
    "version": "v1"
  }
}
```

失败返回：

```json
{
  "ok": false,
  "error": {
    "code": "OPERATION_NOT_FOUND",
    "message": "..."
  },
  "meta": {
    "version": "v1"
  }
}
```

`kind` 建议值：

- `host_help`
- `operation_list`
- `operation_detail`
- `call_result`

## 数据模型

## OperationSummary（list 用）

```json
{
  "name": "ask_question",
  "summary": "Ask any question about a GitHub repository...",
  "required": ["repoName", "question"],
  "input_shape_hint": "object",
  "protocol_kind": "tool"
}
```

## OperationDetail（describe/help 用）

```json
{
  "name": "ask_question",
  "description": "...",
  "input_schema": { "type": "object", "properties": {} },
  "output_schema": null,
  "examples": [
    {
      "cli": "uxc https://mcp.deepwiki.com/mcp ask_question --input-json '{...}'",
      "payload": {}
    }
  ],
  "constraints": {
    "auth_required": false,
    "rate_limit_hint": null
  }
}
```

## 架构改造

1. 解析层拆分为“两阶段解析”。
   - 第一阶段：clap 只解析全局参数（`--format`、缓存、profile 等）。
   - 第二阶段：解析剩余 token 为动态路由（host/help/op/execute）。
2. adapter trait 升级。
   - `operation_help(&self, ...) -> Result<String>` 改为 `describe_operation(&self, ...) -> Result<OperationDetail>`。
   - `list_operations` 保留但补齐 summary/required/input hint。
3. 输出层统一。
   - `output.rs` 从 `call` 专用扩展为全命令通用 envelope。
   - 新增 `text renderer`，只负责展示，不参与语义。
4. 命令分发层新增动态快捷语法。
   - `uxc <host> <operation>` 自动走 execute。
   - `uxc <host> <operation> help|-h` 自动走 describe。

## 变更策略

1. 本次按 breaking change 处理，不保留旧命令。
2. README 和 e2e 全量切换到新语法。
3. CHANGELOG 明确标注不兼容变更点。

## 测试计划

1. CLI 解析测试。
   - `uxc <host> -h` 进入 host help，而非全局帮助（仅在 host 出现时）。
   - `uxc <host> <op> -h` 返回 operation detail。
2. JSON 快照测试。
   - `list/help/describe/execute` 均输出合法 envelope。
3. 协议适配测试。
   - OpenAPI / GraphQL / gRPC / MCP 统一校验 OperationSummary/Detail 字段完整度。
4. e2e smoke 调整。
   - 从 `grep 文本` 升级为 `jq` 校验 JSON 结构。

## 分阶段落地

### Phase 1: 输出协议先行

- 引入全局 `--format`，默认 `json`。
- 将 `list` / `inspect` / `help` 统一到 envelope 输出。

### Phase 2: 描述模型与 adapter 升级

- 引入 `OperationDetail`。
- 全 adapter 实现 `describe_operation`。

### Phase 3: 动态路由

- 支持 `uxc <host> help`、`uxc <host> <op> help|-h`、`uxc <host> <op>`。

### Phase 4: 文档与迁移

- README / e2e / 示例更新。
- 发布 breaking change 说明。

## Issue 拆分（可直接建单）

1. [Feature] CLI JSON-first output mode (`--format`, default json)
   - 范围：全局输出协议、text renderer、统一错误输出。
   - 验收：`list/help/inspect/describe/execute` 默认 JSON。

2. [Feature] Introduce `OperationDetail` and adapter `describe_operation`
   - 范围：trait 变更，四种协议适配实现。
   - 验收：每协议可返回结构化 operation detail。

3. [Feature] Dynamic host-level help (`uxc <host> help` / `uxc <host> -h`)
   - 范围：host 级汇总输出 + 文本/JSON 双渲染。
   - 验收：host help 可展示协议、操作摘要、下一步调用建议。

4. [Feature] Dynamic operation-level help (`uxc <host> <op> help|-h`)
   - 范围：operation 路由与 describe 集成。
   - 验收：可在不写 `call --op-help` 的情况下查看详情。

5. [Feature] Dynamic execution syntax (`uxc <host> <op> --input-json ...`)
   - 范围：动态语法映射到 execute。
   - 验收：可直接执行且不再支持 `call <op>` 旧语法。

6. [Breaking] Remove legacy command path (`call`, `--op-help`)
   - 范围：删除旧语法入口与相关代码分支。
   - 验收：CLI help/README/测试中不再出现旧语法。

7. [Test] JSON contract + parser regression + protocol matrix e2e
   - 范围：单测、快照、e2e smoke（jq 校验）。
   - 验收：CI 中覆盖新语法和四协议。

8. [Docs] README update for progressive CLI and breaking changes
   - 范围：命令示例、默认 JSON 说明、breaking change 说明。
   - 验收：README 与实际 CLI 完全一致。

## 风险

1. clap 默认 `-h` 行为与动态 help 的冲突。
   - 方案：仅当解析到 `<host>` 后，将剩余 `-h` 解释为动态 help。
2. 各协议 schema 信息不对齐（尤其 MCP 的 `anyOf`/复杂 JSON Schema）。
   - 方案：OperationDetail 保留 `input_schema` 原文，summary 仅做 hint。
3. 默认 JSON 可能影响已有人类用户习惯。
   - 方案：提供 `--text`，并在 CHANGELOG 明确 breaking change。
4. 一次性移除旧命令会导致已有本地脚本失效。
   - 方案：在发布说明中给出旧到新语法对照表。

## 完成定义（Definition of Done）

1. 新语法可用：`uxc <host> help`、`uxc <host> <op> help`、`uxc <host> <op> --input-json`。
2. 默认输出 JSON，`--text` 可切换可读格式。
3. 四协议 list/describe/execute 均返回统一 envelope。
4. 旧命令入口已删除（`call`、`--op-help`）。
5. README 与 e2e 全量更新并通过 CI。
