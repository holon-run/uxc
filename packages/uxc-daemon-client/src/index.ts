import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join } from "node:path";
import net from "node:net";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const JSONRPC_VERSION = "2.0";

export interface RuntimeInvokeOptions {
  auth?: string;
  inject_env?: unknown[];
  no_cache?: boolean;
  cache_ttl?: number;
  refresh_schema?: boolean;
  schema_url?: string;
  link_name?: string;
  link_skill?: string;
  link_skill_doc?: string;
  link_skill_path?: string;
  schema_mapping_file?: string;
  daemon_exclusive?: string[];
  daemon_idle_ttl?: number;
  request_headers?: Record<string, string>;
}

export interface RuntimeInvokeResponse {
  protocol: string;
  endpoint: string;
  kind: string;
  operation?: string | null;
  data: unknown;
  duration_ms?: number | null;
  meta: RuntimeMeta;
}

export interface RuntimeMeta {
  schema_involved?: boolean | null;
  cache_source?: string | null;
  cache_age_ms?: number | null;
  cache_stale?: boolean | null;
  cache_fallback?: boolean | null;
  daemon_session_reused?: boolean | null;
  response_status_code?: number | null;
  response_headers?: Record<string, string> | null;
  artifact_truncated?: boolean | null;
  artifact_kind?: string | null;
  artifact_bytes?: number | null;
  artifact_path?: string | null;
  artifact_ref?: string | null;
  artifact_sha256?: string | null;
  [key: string]: unknown;
}

export interface CodegenHostSchemaV1 {
  version: "v1" | string;
  generated_at_unix: number;
  host: {
    id: string;
    endpoint: string;
    protocol: string;
    link_name?: string | null;
  };
  runtime: {
    invoke_options_schema: unknown;
    result_meta_schema: unknown;
    artifact_meta_schema: unknown;
    lifecycle_contract: unknown;
    artifact_contract: unknown;
  };
  operations: CodegenOperationV1[];
}

export interface CodegenOperationV1 {
  id: string;
  display_name: string;
  description?: string | null;
  kind: string;
  input_schema?: unknown;
  output_schema?: unknown;
  result_kind: string;
  execute: boolean;
  help_only: boolean;
  subscribable: boolean;
}

export type RuntimeResult<TData = unknown, TMeta = Record<string, unknown>> = Omit<
  RuntimeInvokeResponse,
  "data" | "meta"
> & {
  data: TData;
  meta: TMeta;
};

export interface SubscriptionEventEnvelope {
  version: string;
  job_id: string;
  seq: number;
  timestamp_unix: number;
  protocol: string;
  source_kind: string;
  event_kind: string;
  data?: unknown;
  meta?: unknown;
}

export interface SubscribeStartResponse {
  job_id: string;
  mode: "stream" | "poll";
  protocol: string;
  endpoint: string;
  sink: string;
  resource_uri?: string | null;
  status: string;
}

export interface SubscribeStopResponse {
  job_id: string;
  stopped: boolean;
}

export interface ManagedSourceSpec {
  endpoint: string;
  operation_id?: string | null;
  args?: Record<string, unknown> | null;
  resource_uri?: string | null;
  read_resource?: boolean;
  transport_hint?:
    | "websocket"
    | "discord_gateway"
    | "slack_socket_mode"
    | "feishu_long_connection"
    | null;
  subprotocols?: string[];
  initial_text_frames?: string[];
  mode: "stream" | "poll";
  poll_config?: PollSubscriptionConfig | null;
  options?: RuntimeInvokeOptions;
}

export interface ManagedSourceEnsureResponse {
  namespace: string;
  source_key: string;
  run_id: string;
  stream_id: string;
  status: string;
  reused: boolean;
  replaced_previous: boolean;
}

export interface ManagedSourceView {
  namespace: string;
  source_key: string;
  run_id: string;
  stream_id: string;
  spec_key: string;
  status: string;
  created_at_unix: number;
  updated_at_unix: number;
  started_at_unix?: number | null;
  stopped_at_unix?: number | null;
  last_error?: string | null;
}

export interface ManagedSourceStopResponse {
  namespace: string;
  source_key: string;
  stopped: boolean;
}

export interface ManagedSourceDeleteResponse {
  namespace: string;
  source_key: string;
  deleted: boolean;
}

export interface ManagedStreamEvent {
  stream_id: string;
  offset: number;
  ingested_at_unix: number;
  raw_payload: unknown;
}

export interface ManagedStreamReadResponse {
  stream_id: string;
  events: ManagedStreamEvent[];
  next_after_offset: number;
  has_more: boolean;
}

export interface ManagedStreamInfo {
  stream_id: string;
  namespace: string;
  source_key: string;
  created_at_unix: number;
  earliest_offset?: number | null;
  latest_offset?: number | null;
  event_count: number;
  retention_max_rows: number;
  retention_max_age_secs: number;
}

export interface ManagedStreamTrimResponse {
  stream_id: string;
  trimmed: number;
}

export interface SubscriptionJobView {
  job_id: string;
  mode: "stream" | "poll";
  endpoint: string;
  protocol: string;
  sink: string;
  resource_uri?: string | null;
  status: string;
  durable: boolean;
  auto_resume: boolean;
  resume_strategy: string;
  created_at_unix: number;
  started_at_unix?: number | null;
  stopped_at_unix?: number | null;
  last_event_at_unix?: number | null;
  last_error?: string | null;
  restart_count: number;
  last_resume_at_unix?: number | null;
  last_resume_error?: string | null;
  reconnect_count: number;
  written_events: number;
}

export interface DaemonStatus {
  running: boolean;
  pid?: number | null;
  socket: string;
  version?: string | null;
  started_at_unix?: number | null;
  request_count: number;
  mcp_stdio_sessions: number;
  mcp_http_sessions: number;
  mcp_reuse_hits: number;
  log_file?: string | null;
}

export interface SubscriptionEventsResponse {
  job_id: string;
  status: string;
  events: SubscriptionEventEnvelope[];
  next_after_seq: number;
  has_more: boolean;
}

export interface UxcDaemonClientOptions {
  socketPath?: string;
  binaryPath?: string;
  autoStart?: boolean;
  connectTimeoutMs?: number;
  requestTimeoutMs?: number;
  env?: NodeJS.ProcessEnv;
}

export interface GenerateTypeScriptClientOptions {
  className?: string;
  packageImport?: string;
  includeSchemaJson?: boolean;
}

export interface SubscribeStartArgs {
  endpoint: string;
  resourceUri?: string;
  operationId?: string;
  args?: Record<string, unknown>;
  mode?: "stream" | "poll";
  pollConfig?: PollSubscriptionConfig;
  options?: RuntimeInvokeOptions;
  sink?: `file:${string}` | "memory:";
  ephemeral?: boolean;
  readResource?: boolean;
  transportHint?:
    | "websocket"
    | "discord_gateway"
    | "slack_socket_mode"
    | "feishu_long_connection";
}

export interface PollSubscriptionConfig {
  interval_secs: number;
  extract_items_pointer: string;
  missing_extract_items_pointer_as_empty?: boolean;
  request_cursor_arg?: string;
  response_cursor_pointer?: string;
  cursor_from_item_pointer?: string;
  cursor_transform?: "increment";
  checkpoint_strategy:
    | {
        type: "cursor_only";
      }
    | {
        type: "item_key";
        item_key_pointer: string;
        seen_window?: number;
      }
    | {
        type: "watermark";
        item_watermark_pointer: string;
        item_tiebreaker_pointer?: string;
        seen_window?: number;
      }
    | {
        type: "content_hash";
        seen_window?: number;
      };
}

export class DaemonRpcError extends Error {
  constructor(
    message: string,
    readonly code: number,
    readonly method: string,
  ) {
    super(message);
    this.name = "DaemonRpcError";
  }
}

export class UxcDaemonClient {
  private readonly socketPath: string;
  private readonly binaryPath: string;
  private readonly autoStart: boolean;
  private readonly connectTimeoutMs: number;
  private readonly requestTimeoutMs: number;
  private readonly env: NodeJS.ProcessEnv;
  private ensureDaemonPromise?: Promise<void>;

  constructor(options: UxcDaemonClientOptions = {}) {
    this.socketPath = options.socketPath ?? defaultSocketPath(options.env);
    this.binaryPath = options.binaryPath ?? "uxc";
    this.autoStart = options.autoStart ?? true;
    this.connectTimeoutMs = options.connectTimeoutMs ?? 2_000;
    this.requestTimeoutMs = options.requestTimeoutMs ?? 20_000;
    this.env = { ...process.env, ...options.env };
  }

  async daemonStatus(): Promise<DaemonStatus> {
    return this.request("daemon.status");
  }

  async daemonSessions(): Promise<unknown[]> {
    return this.request("daemon.sessions");
  }

  async call(args: {
    endpoint: string;
    operation: string;
    payload?: Record<string, unknown> | undefined;
    options?: RuntimeInvokeOptions;
  }): Promise<RuntimeInvokeResponse> {
    return this.request("runtime.invoke", {
      request_id: requestId("call"),
      endpoint: args.endpoint,
      action: "execute",
      operation_id: args.operation,
      args: args.payload ?? null,
      options: normalizeOptions(args.options),
    });
  }

  async codegenSchema(args: {
    endpoint: string;
    options?: RuntimeInvokeOptions;
  }): Promise<CodegenHostSchemaV1> {
    const response = await this.request<RuntimeInvokeResponse>("runtime.invoke", {
      request_id: requestId("codegen"),
      endpoint: args.endpoint,
      action: "codegen_schema",
      operation_id: null,
      args: null,
      options: normalizeOptions(args.options),
    });
    if (response.kind !== "codegen_host_schema") {
      throw new Error(
        `Unexpected codegen response kind '${response.kind}' (expected codegen_host_schema)`,
      );
    }
    assertCodegenHostSchema(response.data);
    return response.data;
  }

  async generateTypeScriptClient(args: {
    endpoint: string;
    options?: RuntimeInvokeOptions;
    emitter?: GenerateTypeScriptClientOptions;
  }): Promise<string> {
    const schema = await this.codegenSchema({
      endpoint: args.endpoint,
      options: args.options,
    });
    return generateTypeScriptClient(schema, args.emitter);
  }

  async subscribeStart(args: SubscribeStartArgs): Promise<SubscribeStartResponse> {
    const mode = args.mode ?? (args.pollConfig ? "poll" : "stream");
    if (mode === "poll" && !args.pollConfig) {
      throw new Error("pollConfig is required when mode is 'poll'");
    }
    if (mode !== "poll" && args.pollConfig) {
      throw new Error("pollConfig is only valid when mode is 'poll'");
    }
    return this.request("subscription.start", {
      request_id: requestId("subscribe"),
      endpoint: args.endpoint,
      sink: args.sink ?? "memory:",
      operation_id: args.operationId ?? null,
      args: args.args ?? null,
      resource_uri: args.resourceUri ?? null,
      read_resource: args.readResource ?? false,
      transport_hint: args.transportHint ?? null,
      subprotocols: [],
      initial_text_frames: [],
      mode,
      poll_config: args.pollConfig ?? null,
      ephemeral: args.ephemeral ?? (args.sink ?? "memory:") === "memory:",
      options: normalizeOptions(args.options),
    });
  }

  async subscribeList(): Promise<SubscriptionJobView[]> {
    return this.request("subscription.list");
  }

  async subscribeStatus(jobId: string): Promise<SubscriptionJobView> {
    return this.request("subscription.status", { job_id: jobId });
  }

  async subscribeStop(jobId: string): Promise<SubscribeStopResponse> {
    return this.request("subscription.stop", { job_id: jobId });
  }

  async subscriptionEvents(args: {
    jobId: string;
    afterSeq?: number;
    limit?: number;
    waitMs?: number;
  }): Promise<SubscriptionEventsResponse> {
    return this.request("subscription.events", {
      job_id: args.jobId,
      after_seq: args.afterSeq ?? 0,
      limit: args.limit ?? 100,
      wait_ms: args.waitMs ?? 15_000,
    });
  }

  async sourceEnsure(args: {
    namespace: string;
    sourceKey: string;
    spec: ManagedSourceSpec;
  }): Promise<ManagedSourceEnsureResponse> {
    return this.request("source.ensure", {
      namespace: args.namespace,
      source_key: args.sourceKey,
      spec: {
        ...args.spec,
        resource_uri: args.spec.resource_uri ?? null,
        read_resource: args.spec.read_resource ?? false,
        transport_hint: args.spec.transport_hint ?? null,
        subprotocols: args.spec.subprotocols ?? [],
        initial_text_frames: args.spec.initial_text_frames ?? [],
        poll_config: args.spec.poll_config ?? null,
        options: normalizeOptions(args.spec.options),
      },
    });
  }

  async sourceStatus(namespace: string, sourceKey: string): Promise<ManagedSourceView> {
    return this.request("source.status", {
      namespace,
      source_key: sourceKey,
    });
  }

  async sourceStop(namespace: string, sourceKey: string): Promise<ManagedSourceStopResponse> {
    return this.request("source.stop", {
      namespace,
      source_key: sourceKey,
    });
  }

  async sourceDelete(namespace: string, sourceKey: string): Promise<ManagedSourceDeleteResponse> {
    return this.request("source.delete", {
      namespace,
      source_key: sourceKey,
    });
  }

  async streamRead(args: {
    streamId: string;
    afterOffset?: number;
    limit?: number;
  }): Promise<ManagedStreamReadResponse> {
    return this.request("stream.read", {
      stream_id: args.streamId,
      after_offset: args.afterOffset ?? 0,
      limit: args.limit ?? 100,
    });
  }

  async streamInfo(streamId: string): Promise<ManagedStreamInfo> {
    return this.request("stream.info", { stream_id: streamId });
  }

  async streamTrim(streamId: string, beforeOffset: number): Promise<ManagedStreamTrimResponse> {
    return this.request("stream.trim", {
      stream_id: streamId,
      before_offset: beforeOffset,
    });
  }

  async *subscribeEvents(
    jobId: string,
    options: {
      afterSeq?: number;
      limit?: number;
      waitMs?: number;
      signal?: AbortSignal;
    } = {},
  ): AsyncIterable<SubscriptionEventEnvelope> {
    let afterSeq = options.afterSeq ?? 0;
    while (true) {
      if (options.signal?.aborted) {
        return;
      }
      const batch = await this.subscriptionEvents({
        jobId,
        afterSeq,
        limit: options.limit,
        waitMs: options.waitMs,
      });
      for (const event of batch.events) {
        afterSeq = event.seq;
        yield event;
      }
      if (batch.status !== "running" && batch.events.length === 0) {
        return;
      }
      if (batch.events.length === 0 && batch.status === "running") {
        continue;
      }
      afterSeq = batch.next_after_seq;
    }
  }

  async request<T>(method: string, params?: unknown): Promise<T> {
    await this.ensureDaemon();
    try {
      return await this.requestOnce<T>(method, params);
    } catch (error) {
      if (!this.autoStart || !isSocketError(error)) {
        throw error;
      }
      await this.ensureDaemon(true);
      return this.requestOnce<T>(method, params);
    }
  }

  private async ensureDaemon(force = false): Promise<void> {
    if (!this.autoStart) {
      return;
    }
    if (!this.ensureDaemonPromise || force) {
      this.ensureDaemonPromise = execFileAsync(this.binaryPath, ["daemon", "start"], {
        env: this.env,
        timeout: this.requestTimeoutMs,
      }).then(() => undefined);
    }
    await this.ensureDaemonPromise;
  }

  private async requestOnce<T>(method: string, params?: unknown): Promise<T> {
    const socket = net.createConnection(this.socketPath);
    const response = await new Promise<T>((resolve, reject) => {
      let timer: NodeJS.Timeout | undefined;
      let buffer: Buffer<ArrayBufferLike> = Buffer.alloc(0);
      let resolved = false;

      const cleanup = () => {
        if (timer) {
          clearTimeout(timer);
        }
        socket.removeAllListeners();
        socket.end();
        socket.destroy();
      };

      const fail = (error: Error) => {
        if (resolved) {
          return;
        }
        resolved = true;
        cleanup();
        reject(error);
      };

      timer = setTimeout(() => fail(new Error(`Timed out waiting for ${method}`)), this.requestTimeoutMs);

      socket.once("error", fail);
      socket.once("connect", () => {
        const body = Buffer.from(
          JSON.stringify({
            jsonrpc: JSONRPC_VERSION,
            id: 1,
            method,
            params: params ?? null,
          }),
        );
        const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`);
        socket.write(Buffer.concat([header, body]));
      });

      socket.on("data", (chunk) => {
        buffer = Buffer.concat([buffer, chunk]);
        const parsed = tryParseFrame(buffer);
        if (!parsed) {
          return;
        }
        buffer = parsed.remaining;
        resolved = true;
        cleanup();
        if (parsed.message.error) {
          reject(
            new DaemonRpcError(
              parsed.message.error.message,
              parsed.message.error.code,
              method,
            ),
          );
          return;
        }
        resolve(parsed.message.result as T);
      });
    });
    return response;
  }
}

export function generateTypeScriptClient(
  schema: CodegenHostSchemaV1,
  options: GenerateTypeScriptClientOptions = {},
): string {
  const packageImport = options.packageImport ?? "@holon-run/uxc-daemon-client";
  const className = sanitizeTypeName(options.className ?? defaultClassName(schema.host.id));
  const methodNames = new Set<string>();
  const typeNames = new Set<string>();
  const operationBlocks: string[] = [];
  const typeBlocks: string[] = [];

  for (const operation of schema.operations) {
    if (!operation.execute || operation.help_only || operation.subscribable) {
      continue;
    }
    const methodName = uniqueName(
      sanitizeMethodName(defaultMethodName(operation.id)),
      methodNames,
    );
    const typeName = uniqueName(
      sanitizeTypeName(`${upperFirst(methodName)}Input`),
      typeNames,
    );
    const selectedInputSchema = selectOperationInputSchema(operation.input_schema);
    const inputType = selectedInputSchema
      ? renderTsTypeFromSchema(selectedInputSchema, 0)
      : "Record<string, unknown>";
    const inputRequired = selectedInputSchema ? hasRequiredInput(selectedInputSchema) : false;
    typeBlocks.push(`export type ${typeName} = ${inputType};`);
    operationBlocks.push(
      [
        `  async ${methodName}(`,
        inputRequired ? `    input: ${typeName},` : `    input?: ${typeName},`,
        `    options: RuntimeInvokeOptions = {},`,
        `  ): Promise<RuntimeResult<unknown>> {`,
        `    return this.client.call({`,
        `      endpoint: this.endpoint,`,
        `      operation: ${JSON.stringify(operation.id)},`,
        `      payload: toRuntimePayload(input),`,
        `      options: { ...this.defaultOptions, ...options },`,
        `    }) as Promise<RuntimeResult<unknown>>;`,
        `  }`,
      ].join("\n"),
    );
  }

  const lines: string[] = [
    `import { UxcDaemonClient, type RuntimeInvokeOptions, type RuntimeResult } from ${JSON.stringify(packageImport)};`,
    "",
    ...typeBlocks,
    "",
    `export interface ${className}Options {`,
    "  client?: UxcDaemonClient;",
    "  endpoint?: string;",
    "  defaultOptions?: RuntimeInvokeOptions;",
    "}",
    "",
    `export class ${className} {`,
    "  readonly client: UxcDaemonClient;",
    "  readonly endpoint: string;",
    "  readonly defaultOptions: RuntimeInvokeOptions;",
    "",
    `  constructor(options: ${className}Options = {}) {`,
    "    this.client = options.client ?? new UxcDaemonClient();",
    `    this.endpoint = options.endpoint ?? ${JSON.stringify(schema.host.endpoint)};`,
    "    this.defaultOptions = options.defaultOptions ?? {};",
    "  }",
    "",
    ...operationBlocks,
    "}",
    "",
    "function toRuntimePayload(input: unknown): Record<string, unknown> | undefined {",
    "  if (input == null) {",
    "    return undefined;",
    "  }",
    "  if (typeof input === \"object\" && !Array.isArray(input)) {",
    "    return input as Record<string, unknown>;",
    "  }",
    "  return { body: input };",
    "}",
  ];

  if (options.includeSchemaJson) {
    lines.push(
      "",
      `export const GENERATED_SCHEMA = ${JSON.stringify(schema, null, 2)} as const;`,
    );
  }

  return `${lines.join("\n")}\n`;
}

function defaultClassName(hostId: string): string {
  return `${sanitizeTypeName(hostId)}Client`;
}

function defaultMethodName(operationId: string): string {
  const openApiMatch = operationId.match(/^([a-z]+):\/(.*)$/i);
  if (openApiMatch) {
    const verb = openApiMatch[1].toLowerCase();
    const path = openApiMatch[2]
      .replace(/\{([^}]+)\}/g, " by $1 ")
      .replace(/[/:_-]+/g, " ");
    return `${verb} ${path}`;
  }
  return operationId.replace(/[/:._-]+/g, " ");
}

function sanitizeMethodName(raw: string): string {
  const tokens = raw
    .replace(/[^a-zA-Z0-9]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (tokens.length === 0) {
    return "invoke";
  }
  const [first, ...rest] = tokens;
  const normalized = [first.toLowerCase(), ...rest.map((token) => upperFirst(token.toLowerCase()))].join(
    "",
  );
  return /^[a-zA-Z_$]/.test(normalized) ? normalized : `op${upperFirst(normalized)}`;
}

function sanitizeTypeName(raw: string): string {
  if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(raw)) {
    return raw;
  }
  const tokens = raw
    .replace(/[^a-zA-Z0-9]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean)
    .map((token) => upperFirst(token));
  const candidate = tokens.join("") || "Generated";
  return /^[A-Za-z_$]/.test(candidate) ? candidate : `T${candidate}`;
}

function upperFirst(input: string): string {
  if (input.length === 0) {
    return input;
  }
  return `${input[0].toUpperCase()}${input.slice(1)}`;
}

function uniqueName(name: string, used: Set<string>): string {
  let candidate = name;
  let idx = 2;
  while (used.has(candidate)) {
    candidate = `${name}${idx}`;
    idx += 1;
  }
  used.add(candidate);
  return candidate;
}

function hasRequiredInput(schema: unknown): boolean {
  if (!schema || typeof schema !== "object") {
    return false;
  }
  const required = (schema as Record<string, unknown>).required;
  return Array.isArray(required) && required.length > 0;
}

function selectOperationInputSchema(inputSchema: unknown): unknown {
  if (!inputSchema || typeof inputSchema !== "object") {
    return undefined;
  }
  const obj = inputSchema as Record<string, unknown>;
  if (obj.kind === "grpc_message" && typeof obj.schema === "object") {
    return obj.schema;
  }
  if (obj.kind === "openrpc_method" && Array.isArray(obj.params)) {
    const properties: Record<string, unknown> = {};
    const required: string[] = [];
    for (const raw of obj.params) {
      if (!raw || typeof raw !== "object") {
        continue;
      }
      const param = raw as Record<string, unknown>;
      const name = typeof param.name === "string" ? param.name : undefined;
      if (!name) {
        continue;
      }
      const schema = typeof param.schema === "object" ? param.schema : {};
      properties[name] = schema;
      if (param.required === true) {
        required.push(name);
      }
    }
    return { type: "object", properties, required };
  }
  if (obj.kind === "openapi_request_body" && obj.content && typeof obj.content === "object") {
    const content = obj.content as Record<string, unknown>;
    const prioritized = [
      "application/json",
      "application/x-www-form-urlencoded",
      "multipart/form-data",
    ];
    for (const mime of prioritized) {
      const entry = content[mime];
      if (entry && typeof entry === "object" && (entry as Record<string, unknown>).schema) {
        return (entry as Record<string, unknown>).schema;
      }
    }
    for (const entry of Object.values(content)) {
      if (entry && typeof entry === "object" && (entry as Record<string, unknown>).schema) {
        return (entry as Record<string, unknown>).schema;
      }
    }
  }
  if (typeof obj.schema === "object" && obj.type == null) {
    return obj.schema;
  }
  return inputSchema;
}

function renderTsTypeFromSchema(schema: unknown, depth: number): string {
  if (!schema || typeof schema !== "object") {
    return "Record<string, unknown>";
  }
  const obj = schema as Record<string, unknown>;
  const typeInfo = resolveSchemaType(obj.type);

  const enumValues = asPrimitiveArray(obj.enum);
  if (enumValues && enumValues.length > 0) {
    return withNullable(
      enumValues.map((value) => JSON.stringify(value)).join(" | "),
      typeInfo.nullable,
    );
  }
  if (Array.isArray(obj.oneOf) && obj.oneOf.length > 0) {
    return withNullable(
      obj.oneOf.map((item) => renderTsTypeFromSchema(item, depth + 1)).join(" | "),
      typeInfo.nullable,
    );
  }
  if (Array.isArray(obj.anyOf) && obj.anyOf.length > 0) {
    return withNullable(
      obj.anyOf.map((item) => renderTsTypeFromSchema(item, depth + 1)).join(" | "),
      typeInfo.nullable,
    );
  }
  const schemaType = typeInfo.base;
  const openApiNullable = obj.nullable === true;
  switch (schemaType) {
    case "string":
      return withNullable("string", typeInfo.nullable || openApiNullable);
    case "integer":
    case "number":
      return withNullable("number", typeInfo.nullable || openApiNullable);
    case "boolean":
      return withNullable("boolean", typeInfo.nullable || openApiNullable);
    case "null":
      return "null";
    case "array": {
      const itemType = renderTsTypeFromSchema(obj.items, depth + 1);
      return withNullable(`Array<${itemType}>`, typeInfo.nullable || openApiNullable);
    }
    case "object": {
      const properties = obj.properties;
      if (!properties || typeof properties !== "object") {
        return withNullable("Record<string, unknown>", typeInfo.nullable || openApiNullable);
      }
      // Bound recursive expansion to keep emitted types readable and finite.
      if (depth > 4) {
        return withNullable("Record<string, unknown>", typeInfo.nullable || openApiNullable);
      }
      const required = new Set(
        Array.isArray(obj.required)
          ? obj.required.filter((item): item is string => typeof item === "string")
          : [],
      );
      const fields = Object.entries(properties as Record<string, unknown>).map(([name, value]) => {
        const optional = required.has(name) ? "" : "?";
        const key = safePropertyName(name);
        const valueType = renderTsTypeFromSchema(value, depth + 1);
        return `${key}${optional}: ${valueType}`;
      });
      if (fields.length === 0) {
        return withNullable("Record<string, unknown>", typeInfo.nullable || openApiNullable);
      }
      return withNullable(`{ ${fields.join("; ")} }`, typeInfo.nullable || openApiNullable);
    }
    default:
      return withNullable("unknown", typeInfo.nullable || openApiNullable);
  }
}

function resolveSchemaType(typeValue: unknown): { base: string | undefined; nullable: boolean } {
  if (typeof typeValue === "string") {
    return {
      base: typeValue,
      nullable: typeValue === "null",
    };
  }
  if (Array.isArray(typeValue)) {
    const nullable = typeValue.some((entry) => entry === "null");
    const nonNull = typeValue.find(
      (entry): entry is string => typeof entry === "string" && entry !== "null",
    );
    return {
      base: nonNull,
      nullable,
    };
  }
  return {
    base: undefined,
    nullable: false,
  };
}

function asPrimitiveArray(value: unknown): Array<string | number | boolean | null> | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const allPrimitive = value.every(
    (item) =>
      item == null ||
      typeof item === "string" ||
      typeof item === "number" ||
      typeof item === "boolean",
  );
  return allPrimitive ? (value as Array<string | number | boolean | null>) : undefined;
}

function safePropertyName(name: string): string {
  return /^[A-Za-z_$][A-Za-z0-9_$]*$/.test(name) ? name : JSON.stringify(name);
}

function withNullable(type: string, nullable: boolean): string {
  if (!nullable || type.includes("null")) {
    return type;
  }
  return `${type} | null`;
}

function toRuntimePayload(input: unknown): Record<string, unknown> | undefined {
  if (input == null) {
    return undefined;
  }
  if (typeof input === "object" && !Array.isArray(input)) {
    return input as Record<string, unknown>;
  }
  return { body: input };
}

function assertCodegenHostSchema(value: unknown): asserts value is CodegenHostSchemaV1 {
  const obj = asRecord(value, "codegen schema");
  assertString(obj.version, "codegen.version");
  assertNumber(obj.generated_at_unix, "codegen.generated_at_unix");

  const host = asRecord(obj.host, "codegen.host");
  assertString(host.id, "codegen.host.id");
  assertString(host.endpoint, "codegen.host.endpoint");
  assertString(host.protocol, "codegen.host.protocol");

  const runtime = asRecord(obj.runtime, "codegen.runtime");
  if (!("invoke_options_schema" in runtime)) {
    throw new Error("Invalid codegen schema: runtime.invoke_options_schema is required");
  }
  if (!("result_meta_schema" in runtime)) {
    throw new Error("Invalid codegen schema: runtime.result_meta_schema is required");
  }
  if (!("artifact_meta_schema" in runtime)) {
    throw new Error("Invalid codegen schema: runtime.artifact_meta_schema is required");
  }
  if (!("lifecycle_contract" in runtime)) {
    throw new Error("Invalid codegen schema: runtime.lifecycle_contract is required");
  }
  if (!("artifact_contract" in runtime)) {
    throw new Error("Invalid codegen schema: runtime.artifact_contract is required");
  }

  if (!Array.isArray(obj.operations)) {
    throw new Error("Invalid codegen schema: operations must be an array");
  }
  for (const [index, operationValue] of obj.operations.entries()) {
    const operation = asRecord(operationValue, `codegen.operations[${index}]`);
    assertString(operation.id, `codegen.operations[${index}].id`);
    assertString(operation.display_name, `codegen.operations[${index}].display_name`);
    assertString(operation.kind, `codegen.operations[${index}].kind`);
    assertString(operation.result_kind, `codegen.operations[${index}].result_kind`);
    assertBoolean(operation.execute, `codegen.operations[${index}].execute`);
    assertBoolean(operation.help_only, `codegen.operations[${index}].help_only`);
    assertBoolean(operation.subscribable, `codegen.operations[${index}].subscribable`);
  }
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`Invalid ${label}: expected object`);
  }
  return value as Record<string, unknown>;
}

function assertString(value: unknown, label: string): asserts value is string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Invalid ${label}: expected non-empty string`);
  }
}

function assertNumber(value: unknown, label: string): asserts value is number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`Invalid ${label}: expected number`);
  }
}

function assertBoolean(value: unknown, label: string): asserts value is boolean {
  if (typeof value !== "boolean") {
    throw new Error(`Invalid ${label}: expected boolean`);
  }
}

function normalizeOptions(options: RuntimeInvokeOptions | undefined): RuntimeInvokeOptions {
  return {
    inject_env: [],
    no_cache: false,
    refresh_schema: false,
    daemon_exclusive: [],
    ...options,
  };
}

function defaultSocketPath(env: NodeJS.ProcessEnv | undefined): string {
  if (env?.XDG_RUNTIME_DIR) {
    return join(env.XDG_RUNTIME_DIR, "uxc", "uxc.sock");
  }
  if (env?.HOME) {
    return join(env.HOME, ".uxc", "daemon", "uxc.sock");
  }
  const label = createHash("sha1")
    .update(env?.USER ?? env?.USERNAME ?? "user")
    .digest("hex")
    .slice(0, 8);
  return join(tmpdir(), `uxc-${label}`, "daemon", "uxc.sock");
}

function requestId(prefix: string): string {
  return `${prefix}-${process.pid}-${Date.now()}`;
}

function isSocketError(error: unknown): boolean {
  return error instanceof Error && /connect|socket|ENOENT|ECONNREFUSED/.test(error.message);
}

function tryParseFrame(buffer: Buffer<ArrayBufferLike>): {
  message: {
    result?: unknown;
    error?: {
      code: number;
      message: string;
    };
  };
  remaining: Buffer<ArrayBufferLike>;
} | null {
  const marker = buffer.indexOf("\r\n\r\n");
  if (marker === -1) {
    return null;
  }
  const header = buffer.subarray(0, marker).toString("utf8");
  const match = header.match(/Content-Length:\s*(\d+)/i);
  if (!match) {
    throw new Error("Missing Content-Length header");
  }
  const bodyLength = Number(match[1]);
  const bodyStart = marker + 4;
  if (buffer.length < bodyStart + bodyLength) {
    return null;
  }
  const body = buffer.subarray(bodyStart, bodyStart + bodyLength).toString("utf8");
  const message = JSON.parse(body) as {
    result?: unknown;
    error?: {
      code: number;
      message: string;
    };
  };
  return {
    message,
    remaining: Buffer.from(buffer.subarray(bodyStart + bodyLength)),
  };
}
