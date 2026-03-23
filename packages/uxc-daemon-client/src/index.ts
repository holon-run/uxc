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
  schema_mapping_file?: string;
  daemon_exclusive?: string[];
  daemon_idle_ttl?: number;
}

export interface RuntimeInvokeResponse {
  protocol: string;
  endpoint: string;
  kind: string;
  operation?: string | null;
  data: unknown;
  duration_ms?: number | null;
  meta: Record<string, unknown>;
}

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

export interface SubscribeStartArgs {
  endpoint: string;
  resourceUri?: string;
  operationId?: string;
  args?: Record<string, unknown>;
  mode?: "stream" | "poll";
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
    payload?: Record<string, unknown>;
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

  async subscribeStart(args: SubscribeStartArgs): Promise<SubscribeStartResponse> {
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
      mode: args.mode ?? "stream",
      poll_config: null,
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
