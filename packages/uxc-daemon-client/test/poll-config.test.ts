import { describe, expect, test } from "vitest";
import { UxcDaemonClient } from "../src/index.js";

describe("poll subscription config", () => {
  test("sourceEnsure forwards poll_config to daemon RPC", async () => {
    const client = new UxcDaemonClient({ autoStart: false });
    const seen: Array<{ method: string; params: Record<string, unknown> }> = [];
    (client as unknown as { request: (method: string, params?: Record<string, unknown>) => Promise<unknown> }).request =
      async (method: string, params?: Record<string, unknown>) => {
        seen.push({ method, params: params ?? {} });
        return {
          namespace: "agentinbox",
          source_key: "github_repo:holon-run/uxc",
          run_id: "run-1",
          stream_id: "stream-1",
          status: "running",
          reused: false,
          replaced_previous: false,
        };
      };

    await client.sourceEnsure({
      namespace: "agentinbox",
      sourceKey: "github_repo:holon-run/uxc",
      spec: {
        endpoint: "https://api.github.com",
        operation_id: "get:/repos/{owner}/{repo}/events",
        args: { owner: "holon-run", repo: "uxc" },
        mode: "poll",
        poll_config: {
          interval_secs: 30,
          extract_items_pointer: "",
          checkpoint_strategy: {
            type: "item_key",
            item_key_pointer: "/id",
          },
        },
      },
    });

    expect(seen).toHaveLength(1);
    expect(seen[0]?.method).toBe("source.ensure");
    expect(seen[0]?.params.spec.poll_config).toEqual({
      interval_secs: 30,
      extract_items_pointer: "",
      checkpoint_strategy: {
        type: "item_key",
        item_key_pointer: "/id",
      },
    });
    expect(seen[0]?.params.spec.mode).toBe("poll");
  });

  test("sourceEnsure preserves poll mode when poll_config is provided", async () => {
    const client = new UxcDaemonClient({ autoStart: false });
    const seen: Array<{ method: string; params: Record<string, unknown> }> = [];
    (client as unknown as { request: (method: string, params?: Record<string, unknown>) => Promise<unknown> }).request =
      async (method: string, params?: Record<string, unknown>) => {
        seen.push({ method, params: params ?? {} });
        return {
          namespace: "agentinbox",
          source_key: "github_repo:holon-run/uxc",
          run_id: "run-2",
          stream_id: "stream-2",
          status: "running",
          reused: false,
          replaced_previous: false,
        };
      };

    await client.sourceEnsure({
      namespace: "agentinbox",
      sourceKey: "github_repo:holon-run/uxc",
      spec: {
        endpoint: "https://api.github.com",
        mode: "poll",
        poll_config: {
          interval_secs: 15,
          extract_items_pointer: "",
          checkpoint_strategy: {
            type: "item_key",
            item_key_pointer: "/id",
          },
        },
      },
    });

    expect(seen[0]?.params.spec.mode).toBe("poll");
    expect(seen[0]?.params.spec.poll_config).toBeTruthy();
  });
});
