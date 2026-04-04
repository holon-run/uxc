import { describe, expect, test } from "vitest";
import { UxcDaemonClient } from "../src/index.js";

describe("poll subscription config", () => {
  test("subscribeStart forwards poll_config to daemon RPC", async () => {
    const client = new UxcDaemonClient({ autoStart: false });
    const seen: Array<{ method: string; params: Record<string, unknown> }> = [];
    (client as unknown as { request: (method: string, params?: Record<string, unknown>) => Promise<unknown> }).request =
      async (method: string, params?: Record<string, unknown>) => {
        seen.push({ method, params: params ?? {} });
        return {
          job_id: "job-1",
          mode: "poll",
          protocol: "openapi",
          endpoint: "https://api.github.com",
          sink: "memory:",
          status: "running",
        };
      };

    await client.subscribeStart({
      endpoint: "https://api.github.com",
      operationId: "get:/repos/{owner}/{repo}/events",
      args: { owner: "holon-run", repo: "uxc" },
      mode: "poll",
      pollConfig: {
        interval_secs: 30,
        extract_items_pointer: "",
        checkpoint_strategy: {
          type: "item_key",
          item_key_pointer: "/id",
        },
      },
      sink: "memory:",
      ephemeral: false,
    });

    expect(seen).toHaveLength(1);
    expect(seen[0]?.method).toBe("subscription.start");
    expect(seen[0]?.params.poll_config).toEqual({
      interval_secs: 30,
      extract_items_pointer: "",
      checkpoint_strategy: {
        type: "item_key",
        item_key_pointer: "/id",
      },
    });
  });
});
