# X API Skill - Usage Patterns

## Link Setup

```bash
command -v x-openapi-cli
uxc link x-openapi-cli https://api.x.com \
  --schema-url https://api.x.com/2/openapi.json
x-openapi-cli -h
```

## OAuth2 PKCE Setup (User Context)

```bash
uxc auth oauth start x-api-user \
  --endpoint https://api.x.com/2 \
  --client-id M3VseG9xOXZXZVREZnNqWk9jN1I6MTpjaQ \
  --redirect-uri http://127.0.0.1:11111/callback \
  --scope users.read \
  --scope tweet.read \
  --scope tweet.write \
  --scope tweet.moderate.write \
  --scope media.write \
  --scope like.read \
  --scope like.write \
  --scope follows.read \
  --scope follows.write \
  --scope list.read \
  --scope list.write \
  --scope timeline.read \
  --scope space.read \
  --scope mute.read \
  --scope mute.write \
  --scope block.read \
  --scope block.write \
  --scope bookmark.read \
  --scope bookmark.write \
  --scope dm.read \
  --scope dm.write \
  --authorization-endpoint https://x.com/i/oauth2/authorize \
  --token-endpoint https://api.x.com/2/oauth2/token
```

Default flow uses the full OAuth2 user scope set except `offline.access`; drop unneeded scopes for least-privilege access when your workflow is narrower. Add `--scope offline.access` only when you need refresh tokens and your X app supports token refresh for this client.

After browser consent:

```bash
uxc auth oauth complete x-api-user \
  --session-id <session_id> \
  --authorization-response '<callback_url>'
```

Bind and verify:

```bash
uxc auth binding add \
  --id x-api-user \
  --host api.x.com \
  --path-prefix /2 \
  --scheme https \
  --credential x-api-user \
  --priority 100

uxc auth binding match https://api.x.com/2
```

## App-only Bearer Setup (Optional)

```bash
uxc auth credential set x-api-app \
  --auth-type bearer \
  --secret-env X_API_BEARER_TOKEN

uxc auth binding add \
  --id x-api-app \
  --host api.x.com \
  --path-prefix /2 \
  --scheme https \
  --credential x-api-app \
  --priority 90
```

## Read Examples

```bash
x-openapi-cli get:/2/users/me

x-openapi-cli get:/2/users/{id}/bookmarks \
  id=<user_id> \
  max_results=20

x-openapi-cli get:/2/users/{id}/timelines/reverse_chronological \
  id=<user_id> \
  max_results=20
```

Incremental reads with `since_id`:

```bash
x-openapi-cli get:/2/users/{id}/timelines/reverse_chronological \
  id=<user_id> \
  since_id=<last_seen_tweet_id> \
  max_results=20
```

## Write Examples (Confirm Before Running)

```bash
x-openapi-cli post:/2/tweets text='hello from uxc'

x-openapi-cli post:/2/tweets '{"text":"hello from uxc"}'
```

Reply:

```bash
x-openapi-cli post:/2/tweets \
  '{"text":"reply from uxc","reply":{"in_reply_to_tweet_id":"<tweet_id>"}}'
```

## Stream Subscribe Example

```bash
uxc subscribe start https://api.x.com/2/tweets/search/stream \
  --auth x-api-app \
  --sink file:$HOME/.uxc/subscriptions/x-search-stream.ndjson
```

## Fallback Equivalence

- `x-openapi-cli <operation> ...` is equivalent to
  `uxc https://api.x.com --schema-url https://api.x.com/2/openapi.json <operation> ...`.
