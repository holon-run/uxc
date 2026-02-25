# DeepWiki Usage Patterns

## Basic Question Flow

1. Ensure the repository is indexed on DeepWiki (visit https://deepwiki.com)
2. Ask a question using `ask_question` tool:
   ```bash
   uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"owner/repo","question":"your question"}'
   ```

## Common Use Cases

### Understand a function or API

```bash
uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"facebook/react","question":"How does useState work?"}'
```

### Find relevant code

```bash
uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"owner/repo","question":"Where is the authentication logic?"}'
```

### Get code review context

```bash
uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"owner/repo","question":"Explain the architecture of this project"}'
```

## Output Handling

Parse the response:

```bash
# Extract the answer text
uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"facebook/react","question":"What is React?"}' | jq -r '.data.content[].text'
```

## Limitations

- Repository must be indexed first
- Max 10 repositories per question
- Some repositories may not be indexed yet
