# Context7 Usage Patterns

This skill defaults to fixed link command `context7-mcp-cli`.
Create it when missing:

```bash
command -v context7-mcp-cli
uxc link context7-mcp-cli mcp.context7.com/mcp
```

## Basic Query Flow

1. First resolve a library name to get library ID:
   ```bash
   context7-mcp-cli resolve-library-id libraryName=package-name query='what you need'
   ```

2. Then use the returned libraryId to query documentation:
   ```bash
   context7-mcp-cli query-docs libraryId=/org/project query='your question'
   ```

## Common Use Cases

### Find React hooks documentation

```bash
context7-mcp-cli resolve-library-id libraryName=react query='useState hook'
```

### Query specific API

```bash
context7-mcp-cli query-docs '{"libraryId":"/reactjs/react.dev","query":"how to use useEffect"}'
```

### Find Node.js fs module docs

```bash
context7-mcp-cli resolve-library-id libraryName=node query='file system'
```

## Output Handling

Parse the response:

```bash
# Extract the answer text
context7-mcp-cli query-docs libraryId=/reactjs/react.dev query=useState | jq -r '.data.content[].text'
```

## Fallback: Explicit JSON Flag

```bash
context7-mcp-cli query-docs --input-json '{"libraryId":"/reactjs/react.dev","query":"useMemo"}'
```

## Fallback Equivalence

- `context7-mcp-cli <operation> ...` is equivalent to `uxc mcp.context7.com/mcp <operation> ...`.
- If link setup is temporarily unavailable, use `uxc mcp.context7.com/mcp ...` as fallback.

## Limitations

- Must resolve library first before querying
- Some libraries may have multiple matches - choose the best one
- Context7 provides up-to-date docs, but coverage varies by library
