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

## Daemon Email RPC Helpers

The package also ships typed helpers for the daemon email RPCs:

```ts
import { UxcDaemonClient } from "@holon-run/uxc-daemon-client";

const runtime = new UxcDaemonClient();

const body = await runtime.emailBodyRead({
  input: { kind: "inline_mime", mimeBase64, originalBytes, complete: true },
});
// body.text holds normalized plain text; body.completeness reports
// "complete" | "partial" | "unverified".

const attachment = await runtime.emailAttachmentGet({
  handle,
  outputPath: "/tmp/report.pdf",
});

await runtime.emailSend({
  smtpUrl: "smtp://smtp.example.com:587",
  from: "bot@example.com",
  to: ["user@example.com"],
  subject: "Hello",
  text: "Hi",
});
```

- `emailBodyRead` normalizes MIME or provider payloads into plain text.
- `emailAttachmentGet` requires a controlled absolute output path and rejects
  attachments larger than `maxBytes` when set.
- `emailReply` sends an SMTP reply using an `email_event` reply handle.

See [Daemon API](../daemon/api.md) for the RPC-level contract.

## Current V1 Boundaries

The first emitter generation keeps a narrow scope:

- generates TypeScript only
- targets normal invoke flows
- preserves artifact and compaction metadata in the result wrapper
- does not yet generate subscription-native helpers

See [Daemon API](../daemon/api.md) for the local runtime contract.
