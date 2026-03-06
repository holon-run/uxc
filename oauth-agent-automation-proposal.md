# OAuth Agent Automation Support

## Problem Statement

When `uxc auth oauth login` is called with `authorization_code` flow, it displays an authorization URL and then blocks waiting for user input via stdin. This creates a problem for agent-driven workflows (like Claude Code):

- Bash tool calls are non-interactive and cannot maintain stdin sessions
- Each Bash invocation is a separate process
- Agent cannot paste the callback URL while the command is waiting for input

## Current Behavior

From `src/auth/oauth.rs:287-297`:
```rust
let input = authorization_code
    .or_else(read_authorization_code_from_stdin)
    .ok_or_else(|| {
        UxcError::OAuthTokenExchangeFailed(
            "Authorization code is required to continue".to_string(),
        )
    })?;
```

The function `read_authorization_code_from_stdin()` performs blocking I/O, which doesn't work in agent workflows.

## Affected Workflows

- Agent-initiated OAuth logins (Claude Code, other AI agents)
- Automated OAuth setup in scripts
- Non-interactive environments where authorization codes are obtained through other means

## Proposed Solutions

### Option 1: Non-Interactive Flag (Recommended)

Add a `--non-interactive` or `--wait-for-code` flag that changes the behavior:

```bash
# Start OAuth flow and exit with URL
uxc auth oauth login discord-user \
  --endpoint https://discord.com/api/v10/oauth2/token \
  --flow authorization_code \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify guilds" \
  --non-interactive

# Output: JSON with authorization URL and session ID
# {
#   "authorization_url": "https://...",
#   "session_id": "abc123",
#   "state": "..."
# }

# Complete the flow with the callback URL
uxc auth oauth complete discord-user \
  --callback-url "http://127.0.0.1:11111/callback?code=..." \
  --session-id "abc123"
```

**Pros:**
- Clean separation between initiation and completion
- Agent can handle the two-step process easily
- Session ID ensures proper state management

**Cons:**
- Requires new `oauth complete` subcommand
- More complex state management

### Option 2: Direct Code/Callback URL Parameter

Enhance the existing `--authorization-code` parameter to also accept full callback URLs:

```bash
# Agent generates authorization URL separately
# User completes authorization in browser
# Agent receives callback URL and completes flow

uxc auth oauth login discord-user \
  --endpoint https://discord.com/api/v10/oauth2/token \
  --flow authorization_code \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify guilds" \
  --callback-url "http://127.0.0.1:11111/callback?code=..." \
  --state "..."
```

**Pros:**
- Single command completes the flow
- Leverages existing parameter structure

**Cons:**
- Requires state to be extracted from initial authorization URL
- Still two-step process but less explicit

### Option 3: File-Based State Persistence

Save PKCE state to a file between invocations:

```bash
# Step 1: Generate authorization URL and save state
uxc auth oauth login discord-user \
  --endpoint https://discord.com/api/v10/oauth2/token \
  --flow authorization_code \
  --client-id 1479302369723285736 \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope "identify guilds" \
  --state-file /tmp/discord_oauth_state.json

# Output: {
#   "authorization_url": "https://...",
#   "state_file": "/tmp/discord_oauth_state.json"
# }

# Step 2: Complete with callback URL
uxc auth oauth complete discord-user \
  --state-file /tmp/discord_oauth_state.json \
  --callback-url "http://127.0.0.1:11111/callback?code=..."
```

**Pros:**
- State persistence across invocations
- Can support resumable flows

**Cons:**
- File I/O and cleanup complexity
- Security concerns with temporary files

### Option 4: Stdin/Stdout JSON Protocol

Use structured JSON for non-interactive mode:

```bash
# Initiation
echo '{"mode": "initiate"}' | uxc auth oauth login discord-user --json

# Response: {
#   "status": "awaiting_callback",
#   "authorization_url": "https://...",
#   "state": "..."
# }

# Completion
echo '{
  "mode": "complete",
  "callback_url": "http://127.0.0.1:11111/callback?code=...",
  "state": "..."
}' | uxc auth oauth login discord-user --json
```

**Pros:**
- Clean protocol design
- Easy to parse programmatically

**Cons:**
- Requires protocol documentation
- Different interaction model

## Recommendation

**Option 1 (Non-Interactive Flag)** is recommended because:

1. **Clear UX**: Explicit two-step process matches the mental model
2. **Agent-friendly**: Each step is a separate, atomic operation
3. **Backward compatible**: Interactive mode remains unchanged
4. **State management**: Session ID provides clean state handling
5. **Error handling**: Can provide clear errors for missing/expired sessions

## Implementation Notes

- Store pending OAuth sessions in `~/.uxc/oauth_sessions/`
- Include expiry time for session cleanup
- Support `--session-timeout` parameter (default: 10 minutes)
- Add `uxc auth oauth list-sessions` command
- Add `uxc auth oauth cleanup-sessions` command

## Related

- Discord OAuth2 user authentication testing
- Agent-driven credential management workflows
- Non-interactive CI/CD OAuth flows
