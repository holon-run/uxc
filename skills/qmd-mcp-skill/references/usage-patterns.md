# QMD MCP Usage Patterns

This skill defaults to fixed link command `qmd-mcp-cli`.

Create it when missing:

```bash
command -v qmd-mcp-cli
uxc link --daemon-idle-ttl 0 qmd-mcp-cli "qmd mcp"
```

If `qmd` depends on shell setup, wrap the command explicitly:

```bash
uxc link --daemon-idle-ttl 0 qmd-mcp-cli "/bin/bash -lc 'export NVM_DIR=$HOME/.nvm; . $NVM_DIR/nvm.sh; nvm use 23 >/dev/null; export CUDA_PATH=/usr/local/cuda-11.6; export CUDA_HOME=/usr/local/cuda-11.6; export CUDACXX=/usr/local/cuda-11.6/bin/nvcc; export LD_LIBRARY_PATH=/usr/local/cuda-11.6/lib64:${LD_LIBRARY_PATH:-}; export NODE_LLAMA_CPP_CMAKE_OPTION_CMAKE_CUDA_ARCHITECTURES=86; export NODE_LLAMA_CPP_GPU=cuda; qmd mcp'"
```

Inspect the linked tools first:

```bash
qmd-mcp-cli -h
qmd-mcp-cli query -h
qmd-mcp-cli get -h
qmd-mcp-cli multi_get -h
qmd-mcp-cli status -h
```

## Fast Interactive Search

Prefer a typed `lex` query first:

```bash
qmd-mcp-cli query '{"searches":[{"type":"lex","query":"\"execution layer\" MCP CLI -migration"}],"collections":["workspace"],"limit":5,"intent":"Find the article about the missing execution surface for agents"}'
```

Add `vec` when the query is more semantic:

```bash
qmd-mcp-cli query '{"searches":[{"type":"lex","query":"\"Bitcoin L2\" DA rollup"},{"type":"vec","query":"How should Bitcoin layer 2 relate to DA and rollup architecture?"}],"collections":["workspace"],"limit":5,"intent":"Find articles discussing Bitcoin L2, DA, and rollup design"}'
```

Use `hyde` only when recall is still weak:

```bash
qmd-mcp-cli query '{"searches":[{"type":"lex","query":"\"connection pool\" timeout -redis"},{"type":"vec","query":"Why do database connections time out under load?"},{"type":"hyde","query":"Connection pool exhaustion occurs when all connections are busy and requests must wait. Under high concurrency, long-running queries can cause timeout cascades."}],"limit":5}'
```

## Retrieve Documents After Search

Fetch one chosen document:

```bash
qmd-mcp-cli get file=workspace/public/how-to-define-bitcoin-l2/readme.md
```

Fetch multiple known candidates:

```bash
qmd-mcp-cli multi_get '{"pattern":"workspace/public/how-to-define-bitcoin-l2/readme.md,workspace/public/how-bitcoin-layer2-should-work/readme.md","maxBytes":32768}'
```

## Session Reuse Checks

Check that `uxc` is reusing the stdio session:

```bash
uxc daemon status
uxc daemon sessions
```

Recommended signs:

- `mcp_stdio_sessions` is non-zero
- `mcp_reuse_hits` increases after repeated calls
- the `qmd-mcp-cli` session appears as `ready`

## Fallback Equivalence

- `qmd-mcp-cli <operation> ...` is equivalent to `uxc "qmd mcp" <operation> ...` when `qmd` already runs correctly in the current shell.
- If shell initialization or GPU env exports are required, use the same wrapped shell command directly with `uxc "<wrapped-host>" ...` as fallback.

## Limitations

- The first daemon-backed call can still be slow because QMD may warm model state on first real request.
- `lex` is usually the best default for low latency.
- `vec` and especially `hyde` raise latency and should be added intentionally.
- This skill assumes the QMD index already exists and is healthy; it does not replace `qmd update` or `qmd embed`.
