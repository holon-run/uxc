# Generated TypeScript Clients

UXC ships a daemon client package for local app integration:

```bash
npm install @holon-run/uxc-daemon-client
```

The package can also generate typed TypeScript clients from the exported
host-scoped runtime codegen schema.

## Export Host-Scoped Codegen Input

From CLI:

```bash
uxc <host> --codegen-schema
```

This returns a `codegen_host_schema` envelope that describes:

- host identity
- runtime capabilities
- operation input and output schemas
- artifact and compaction metadata boundaries

## Generate A Typed Client

From Node/TypeScript:

```ts
import { UxcDaemonClient } from "@holon-run/uxc-daemon-client";

const runtime = new UxcDaemonClient();
const source = await runtime.generateTypeScriptClient({
  endpoint: "https://petstore3.swagger.io/api/v3",
  options: { no_cache: true },
  emitter: { className: "PetstoreClient" },
});
```

The generated client targets the UXC daemon/runtime surface rather than a
protocol-native direct client.

## Current V1 Boundaries

The first emitter generation keeps a narrow scope:

- generates TypeScript only
- targets normal invoke flows
- preserves artifact and compaction metadata in the result wrapper
- does not yet generate subscription-native helpers

See [Daemon API](../daemon/api.md) for the local runtime contract.
