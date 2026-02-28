# DeepWiki Usage Patterns

This skill defaults to fixed link command `deepwiki-mcp-cli`.
Create it when missing:

```bash
command -v deepwiki-mcp-cli
uxc link deepwiki-mcp-cli mcp.deepwiki.com/mcp
```

## Basic Question Flow

1. Ensure the repository is indexed on DeepWiki (visit https://deepwiki.com)
2. Ask a question using `ask_question` tool:
   ```bash
   deepwiki-mcp-cli ask_question repoName=owner/repo question='your question'
   ```

## Common Use Cases

### Understand a function or API

```bash
deepwiki-mcp-cli ask_question repoName=facebook/react question='How does useState work?'
```

### Find relevant code

```bash
deepwiki-mcp-cli ask_question '{"repoName":"owner/repo","question":"Where is the authentication logic?"}'
```

### Get code review context

```bash
deepwiki-mcp-cli ask_question repoName=owner/repo question='Explain the architecture of this project'
```

## Output Handling

Parse the response:

```bash
# Extract the answer text
deepwiki-mcp-cli ask_question repoName=facebook/react question='What is React?' | jq -r '.data.content[].text'
```

## Fallback Equivalence

- `deepwiki-mcp-cli <operation> ...` is equivalent to `uxc mcp.deepwiki.com/mcp <operation> ...`.
- If link setup is temporarily unavailable, use `uxc mcp.deepwiki.com/mcp ...` as fallback.

## Limitations

- Repository must be indexed first
- Max 10 repositories per question
- Some repositories may not be indexed yet
