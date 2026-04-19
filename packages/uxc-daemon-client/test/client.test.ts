import { mkdtemp, rm } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { WebSocketServer } from "ws";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { DaemonRpcError, generateTypeScriptClient, UxcDaemonClient, type CodegenHostSchemaV1 } from "../src/index.js";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const execFileAsync = promisify(execFile);
const thisDir = dirname(fileURLToPath(import.meta.url));
const defaultUxcBin = resolve(thisDir, "..", "..", "..", "target", "debug", "uxc");

describe("UxcDaemonClient", () => {
  let homeDir = "";
  let runtimeDir = "";
  let client: UxcDaemonClient;
  let uxcBin = process.env.UXC_BIN ?? defaultUxcBin;

  beforeAll(async () => {
    homeDir = await mkdtemp(join(tmpdir(), "uxc-sdk-home-"));
    runtimeDir = join(homeDir, "runtime");
    client = new UxcDaemonClient({
      binaryPath: uxcBin,
      env: {
        HOME: homeDir,
        USERPROFILE: homeDir,
        XDG_RUNTIME_DIR: runtimeDir,
      },
    });
  });

  afterAll(async () => {
    await execFileAsync(uxcBin, ["daemon", "stop"], {
      env: {
        HOME: homeDir,
        USERPROFILE: homeDir,
        XDG_RUNTIME_DIR: runtimeDir,
      },
    }).catch(() => undefined);
    await rm(homeDir, { recursive: true, force: true });
  });

  test("daemonStatus autostarts the daemon", async () => {
    const status = await client.daemonStatus();
    expect(status.running).toBe(true);
    expect(status.socket).toContain("uxc.sock");
  });

  test("request uses the live socket before attempting daemon autostart", async () => {
    const stub = new UxcDaemonClient();
    let startCalls = 0;
    (stub as unknown as { requestOnce: () => Promise<unknown> }).requestOnce = async () => ({
      running: true,
    });
    (stub as unknown as { startDaemonProcess: () => Promise<void> }).startDaemonProcess = async () => {
      startCalls += 1;
    };

    const result = await stub.request("daemon.status");
    expect(result).toEqual({ running: true });
    expect(startCalls).toBe(0);
  });

  test("request autostarts only after a socket error and retries once", async () => {
    const stub = new UxcDaemonClient();
    let requestCalls = 0;
    let startCalls = 0;
    (stub as unknown as { requestOnce: () => Promise<unknown> }).requestOnce = async () => {
      requestCalls += 1;
      if (requestCalls === 1) {
        throw new Error("connect ENOENT /tmp/uxc.sock");
      }
      return { running: true };
    };
    (stub as unknown as { startDaemonProcess: () => Promise<void> }).startDaemonProcess = async () => {
      startCalls += 1;
    };

    const result = await stub.request("daemon.status");
    expect(result).toEqual({ running: true });
    expect(requestCalls).toBe(2);
    expect(startCalls).toBe(1);
  });

  test("request does not autostart for non-socket errors", async () => {
    const stub = new UxcDaemonClient();
    let startCalls = 0;
    (stub as unknown as { requestOnce: () => Promise<unknown> }).requestOnce = async () => {
      throw new DaemonRpcError("method failed", -32000, "daemon.status");
    };
    (stub as unknown as { startDaemonProcess: () => Promise<void> }).startDaemonProcess = async () => {
      startCalls += 1;
    };

    await expect(stub.request("daemon.status")).rejects.toThrow(/method failed/);
    expect(startCalls).toBe(0);
  });

  test("failed autostart does not poison later socket recovery", async () => {
    const stub = new UxcDaemonClient();
    let requestCalls = 0;
    let startCalls = 0;
    (stub as unknown as { requestOnce: () => Promise<unknown> }).requestOnce = async () => {
      requestCalls += 1;
      if (requestCalls <= 2) {
        throw new Error("connect ECONNREFUSED /tmp/uxc.sock");
      }
      return { running: true };
    };
    (stub as unknown as { startDaemonProcess: () => Promise<void> }).startDaemonProcess = async () => {
      startCalls += 1;
      if (startCalls === 1) {
        throw new Error("daemon start failed");
      }
    };

    await expect(stub.request("daemon.status")).rejects.toThrow(/daemon start failed/);
    const result = await stub.request("daemon.status");
    expect(result).toEqual({ running: true });
    expect(startCalls).toBe(2);
    expect(requestCalls).toBe(3);
  });

  test("daemonSessionKill sends daemon.session.kill with session key", async () => {
    const stub = new UxcDaemonClient({ autoStart: false });
    const calls: Array<{ method: string; params: unknown }> = [];
    (stub as unknown as { request: (method: string, params?: unknown) => Promise<unknown> }).request = async (
      method,
      params,
    ) => {
      calls.push({ method, params });
      return {
        session_key: "stdio:0123456789abcdef",
        child_pid: 4242,
        killed: true,
      };
    };

    const response = await stub.daemonSessionKill("stdio:0123456789abcdef");
    expect(response).toEqual({
      session_key: "stdio:0123456789abcdef",
      child_pid: 4242,
      killed: true,
    });
    expect(calls).toEqual([
      {
        method: "daemon.session.kill",
        params: { session_key: "stdio:0123456789abcdef" },
      },
    ]);
  });

  test("call executes OpenAPI operations without CLI envelope parsing", async () => {
    const server = createOpenApiServer();
    await server.start();
    try {
      const response = await client.call({
        endpoint: server.baseUrl,
        operation: "get:/health",
        options: { no_cache: true },
      });
      expect(response.kind).toBe("call_result");
      expect((response.data as { ok: boolean }).ok).toBe(true);
    } finally {
      await server.stop();
    }
  });

  test("call defaults artifact compaction off for daemon client consumers", async () => {
    const stub = new UxcDaemonClient({ autoStart: false });
    const calls: Array<{ method: string; params: any }> = [];
    (stub as unknown as { request: (method: string, params?: unknown) => Promise<unknown> }).request = async (
      method,
      params,
    ) => {
      calls.push({ method, params });
      return {
        protocol: "openapi",
        endpoint: "https://example.test",
        kind: "call_result",
        operation: "get:/health",
        data: { ok: true },
        meta: {},
      };
    };

    await stub.call({
      endpoint: "https://example.test",
      operation: "get:/health",
    });

    expect(calls).toHaveLength(1);
    expect(calls[0]?.method).toBe("runtime.invoke");
    expect(calls[0]?.params.options.artifact_compaction).toBe(false);
  });

  test("call preserves explicit artifact compaction override", async () => {
    const stub = new UxcDaemonClient({ autoStart: false });
    const calls: Array<{ method: string; params: any }> = [];
    (stub as unknown as { request: (method: string, params?: unknown) => Promise<unknown> }).request = async (
      method,
      params,
    ) => {
      calls.push({ method, params });
      return {
        protocol: "openapi",
        endpoint: "https://example.test",
        kind: "call_result",
        operation: "get:/health",
        data: { ok: true },
        meta: {},
      };
    };

    await stub.call({
      endpoint: "https://example.test",
      operation: "get:/health",
      options: { artifact_compaction: true },
    });

    expect(calls).toHaveLength(1);
    expect(calls[0]?.params.options.artifact_compaction).toBe(true);
  });

  test("codegenSchema exports host-scoped runtime codegen input", async () => {
    const server = createOpenApiServer();
    await server.start();
    try {
      const schema = await client.codegenSchema({
        endpoint: server.baseUrl,
        options: { no_cache: true },
      });
      expect(schema.version).toBe("v1");
      expect(schema.host.endpoint).toBe(server.baseUrl);
      expect(schema.host.protocol).toBe("openapi");
      expect(schema.runtime).toHaveProperty("invoke_options_schema");
      expect(schema.runtime).toHaveProperty("lifecycle_contract");
      expect(schema.runtime).toHaveProperty("artifact_contract");
      expect(schema.operations.length).toBeGreaterThan(0);
      expect(schema.operations[0]?.id).toBe("get:/health");
    } finally {
      await server.stop();
    }
  });

  test("generateTypeScriptClient emits compilable host client source", async () => {
    const schema: CodegenHostSchemaV1 = {
      version: "v1",
      generated_at_unix: Math.floor(Date.now() / 1000),
      host: {
        id: "petstore",
        endpoint: "https://example.test/api",
        protocol: "openapi",
      },
      runtime: {
        invoke_options_schema: {},
        result_meta_schema: {},
        artifact_meta_schema: {},
        lifecycle_contract: {},
        artifact_contract: {},
      },
      operations: [
        {
          id: "get:/health",
          display_name: "GET /health",
          kind: "execute",
          input_schema: {
            type: "object",
            properties: {
              verbose: { type: "boolean" },
            },
            required: [],
          },
          output_schema: null,
          result_kind: "call_result",
          execute: true,
          help_only: false,
          subscribable: false,
        },
      ],
    };

    const source = generateTypeScriptClient(schema, {
      className: "PetstoreClient",
    });
    expect(source).toContain("export class PetstoreClient");
    expect(source).toContain("async getHealth(");
    expect(source).toContain("payload: toRuntimePayload(input)");

    const transpiled = ts.transpileModule(source, {
      reportDiagnostics: true,
      compilerOptions: {
        target: ts.ScriptTarget.ES2022,
        module: ts.ModuleKind.ESNext,
      },
    });
    const diagnostics = transpiled.diagnostics ?? [];
    const errors = diagnostics.filter((diag) => diag.category === ts.DiagnosticCategory.Error);
    expect(errors).toHaveLength(0);
  });

  test("generateTypeScriptClient skips subscribable operations in v1 emitter", () => {
    const schema: CodegenHostSchemaV1 = {
      version: "v1",
      generated_at_unix: Math.floor(Date.now() / 1000),
      host: {
        id: "graphql-demo",
        endpoint: "https://example.test/graphql",
        protocol: "graphql",
      },
      runtime: {
        invoke_options_schema: {},
        result_meta_schema: {},
        artifact_meta_schema: {},
        lifecycle_contract: {},
        artifact_contract: {},
      },
      operations: [
        {
          id: "subscription/newMessage",
          display_name: "subscription newMessage",
          kind: "subscription",
          input_schema: { type: "object", properties: {} },
          output_schema: null,
          result_kind: "subscription_event",
          execute: true,
          help_only: false,
          subscribable: true,
        },
      ],
    };
    const source = generateTypeScriptClient(schema, { className: "GraphqlDemoClient" });
    expect(source).not.toContain("subscription/newMessage");
  });

  test("generateTypeScriptClient keeps nullability from union schema types", () => {
    const schema: CodegenHostSchemaV1 = {
      version: "v1",
      generated_at_unix: Math.floor(Date.now() / 1000),
      host: {
        id: "nullable-demo",
        endpoint: "https://example.test/rpc",
        protocol: "jsonrpc",
      },
      runtime: {
        invoke_options_schema: {},
        result_meta_schema: {},
        artifact_meta_schema: {},
        lifecycle_contract: {},
        artifact_contract: {},
      },
      operations: [
        {
          id: "get:user",
          display_name: "get user",
          kind: "execute",
          input_schema: {
            type: "object",
            properties: {
              nickname: { type: ["string", "null"] },
            },
          },
          output_schema: null,
          result_kind: "call_result",
          execute: true,
          help_only: false,
          subscribable: false,
        },
      ],
    };
    const source = generateTypeScriptClient(schema, { className: "NullableClient" });
    expect(source).toContain("nickname?: string | null");
  });

  test("codegenSchema rejects invalid response kind", async () => {
    const stub = new UxcDaemonClient({ autoStart: false });
    (stub as unknown as { request: (...args: unknown[]) => Promise<unknown> }).request = async () => ({
      protocol: "openapi",
      endpoint: "https://example.test",
      kind: "call_result",
      operation: null,
      data: {},
      duration_ms: 1,
      meta: {},
    });
    await expect(
      stub.codegenSchema({
        endpoint: "https://example.test",
      }),
    ).rejects.toThrow(/Unexpected codegen response kind/i);
  });

  test("codegenSchema rejects malformed codegen payload", async () => {
    const stub = new UxcDaemonClient({ autoStart: false });
    (stub as unknown as { request: (...args: unknown[]) => Promise<unknown> }).request = async () => ({
      protocol: "openapi",
      endpoint: "https://example.test",
      kind: "codegen_host_schema",
      operation: null,
      data: { version: "v1", host: {}, runtime: {}, operations: [] },
      duration_ms: 1,
      meta: {},
    });
    await expect(
      stub.codegenSchema({
        endpoint: "https://example.test",
      }),
    ).rejects.toThrow(/Invalid codegen/i);
  });

  test("managed source stream lifecycle works through daemon client methods", async () => {
    const server = await createWebSocketServer();
    const namespace = "sdk-test";
    const sourceKey = `websocket:${Date.now()}`;
    try {
      const ensured = await client.sourceEnsure({
        namespace,
        sourceKey,
        spec: {
          endpoint: server.endpoint,
          mode: "stream",
          transport_hint: "websocket",
        },
      });

      expect(ensured.namespace).toBe(namespace);
      expect(ensured.source_key).toBe(sourceKey);
      expect(ensured.stream_id).toBeTruthy();
      expect(ensured.run_id).toBeTruthy();

      const status = await client.sourceStatus(namespace, sourceKey);
      expect(status.namespace).toBe(namespace);
      expect(status.source_key).toBe(sourceKey);
      expect(status.stream_id).toBe(ensured.stream_id);

      const listed = await client.sourceList();
      expect(
        listed.some(
          (entry) =>
            entry.namespace === namespace &&
            entry.source_key === sourceKey &&
            entry.stream_id === ensured.stream_id,
        ),
      ).toBe(true);

      const initialRead = await waitForManagedStreamEvent(client, ensured.stream_id);
      expect(initialRead.events[0]?.raw_payload).toEqual({ value: 42 });

      const info = await client.streamInfo(ensured.stream_id);
      expect(info.stream_id).toBe(ensured.stream_id);
      expect(info.namespace).toBe(namespace);
      expect(info.source_key).toBe(sourceKey);
      expect(info.event_count).toBeGreaterThanOrEqual(1);

      const stopped = await client.sourceStop(namespace, sourceKey);
      expect(stopped.stopped).toBe(true);

      const stoppedStatus = await client.sourceStatus(namespace, sourceKey);
      expect(stoppedStatus.status).toBe("stopped");

      const deleted = await client.sourceDelete(namespace, sourceKey);
      expect(deleted.deleted).toBe(true);

      const preserved = await client.streamRead({
        streamId: ensured.stream_id,
        afterOffset: 0,
        limit: 10,
      });
      expect(preserved.events.length).toBeGreaterThanOrEqual(1);

      const trimmed = await client.streamTrim(ensured.stream_id, 1_000_000);
      expect(trimmed.trimmed).toBeGreaterThanOrEqual(1);

      const afterTrim = await client.streamRead({
        streamId: ensured.stream_id,
        afterOffset: 0,
        limit: 10,
      });
      expect(afterTrim.events).toHaveLength(0);
    } finally {
      await server.stop();
    }
  });
});

function createOpenApiServer() {
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    if (req.url === "/openapi.json") {
      res.setHeader("content-type", "application/json");
      res.end(
        JSON.stringify({
          openapi: "3.0.0",
          info: { title: "test", version: "1.0.0" },
          paths: {
            "/health": {
              get: {
                operationId: "getHealth",
                responses: {
                  "200": { description: "ok" },
                },
              },
            },
          },
        }),
      );
      return;
    }
    if (req.url === "/health") {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ ok: true }));
      return;
    }
    res.statusCode = 404;
    res.end();
  });

  return {
    baseUrl: "",
    async start() {
      await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", () => resolve()));
      const address = server.address();
      if (typeof address === "string" || address == null) {
        throw new Error("Unexpected server address");
      }
      this.baseUrl = `http://127.0.0.1:${address.port}`;
    },
    async stop() {
      await new Promise<void>((resolve, reject) => server.close((err) => (err ? reject(err) : resolve())));
    },
  };
}

async function createWebSocketServer() {
  const httpServer = createServer();
  const wsServer = new WebSocketServer({ server: httpServer });
  wsServer.on("connection", (socket) => {
    socket.send(JSON.stringify({ value: 42 }));
  });
  await new Promise<void>((resolve) => httpServer.listen(0, "127.0.0.1", () => resolve()));
  const address = httpServer.address();
  if (typeof address === "string" || address == null) {
    throw new Error("Unexpected websocket address");
  }
  return {
    endpoint: `ws://127.0.0.1:${address.port}`,
    async stop() {
      await new Promise<void>((resolve, reject) => {
        wsServer.close((err) => (err ? reject(err) : resolve()));
      });
      await new Promise<void>((resolve, reject) => {
        httpServer.close((err) => (err ? reject(err) : resolve()));
      });
    },
  };
}

async function waitForManagedStreamEvent(
  client: UxcDaemonClient,
  streamId: string,
  timeoutMs = 5_000,
): Promise<Awaited<ReturnType<UxcDaemonClient["streamRead"]>>> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const page = await client.streamRead({
      streamId,
      afterOffset: 0,
      limit: 10,
    });
    if (page.events.length > 0) {
      return page;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for managed stream event on ${streamId}`);
}
