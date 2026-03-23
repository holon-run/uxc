import { mkdtemp, rm } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WebSocketServer } from "ws";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { UxcDaemonClient } from "../src/index.js";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

describe("UxcDaemonClient", () => {
  let homeDir = "";
  let runtimeDir = "";
  let client: UxcDaemonClient;
  let uxcBin = process.env.UXC_BIN ?? join(process.cwd(), "target", "debug", "uxc");

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

  test("subscribeStart + subscribeEvents streams memory sink events", async () => {
    const server = await createWebSocketServer();
    try {
      const started = await client.subscribeStart({
        endpoint: server.endpoint,
        sink: "memory:",
        transportHint: "websocket",
      });
      const seenKinds: string[] = [];
      for await (const event of client.subscribeEvents(started.job_id, { waitMs: 1000 })) {
        seenKinds.push(event.event_kind);
        if (event.event_kind === "data") {
          break;
        }
      }
      expect(seenKinds).toContain("open");
      expect(seenKinds).toContain("data");
      const stop = await client.subscribeStop(started.job_id);
      expect(stop.stopped).toBe(true);
      const status = await client.subscribeStatus(started.job_id);
      expect(status.status).toBe("stopped");
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
