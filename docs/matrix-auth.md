# Matrix Auth in UXC

This note records the decision for issue #274.

## Conclusion

Matrix can fit `uxc`'s existing two-step auth/session model without adding a Matrix-specific
session mechanism, as long as the homeserver exposes Matrix OAuth metadata at
`/_matrix/client/v1/auth_metadata`.

For those homeservers, the recommended flow is:

1. `uxc auth oauth start <credential_id> --endpoint <matrix_client_base> ...`
2. user completes authorization in the browser
3. `uxc auth oauth complete <credential_id> --session-id <id> ...`
4. `uxc` persists the resulting token credential and bindings work as usual

## Why This Fits

The current OAuth two-step flow already provides the right primitives:

- pending session persistence between CLI invocations
- browser/external completion outside the CLI process
- completion using stored state and PKCE verifier
- token-based credential persistence as the final runtime credential

That is sufficient for Matrix OAuth-aware homeservers such as `matrix.org`.

## Current Boundary

This design does not cover Matrix-specific fallback flows such as:

- `m.login.password`
- `m.login.sso`
- `m.login.token`

If Matrix OAuth discovery turns out to be insufficient in practice, those fallback flows should be
evaluated separately as provider-specific auth on top of new generic auth/session primitives.

## Implementation Scope for #274

- add Matrix OAuth metadata discovery to existing `auth oauth`
- keep the current OAuth session storage model unchanged
- document Matrix OAuth-aware setup in the Matrix skill
