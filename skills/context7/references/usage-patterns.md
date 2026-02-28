# Context7 Usage Patterns

## Basic Query Flow

1. First resolve a library name to get library ID:
   ```bash
   uxc https://mcp.context7.com/mcp resolve-library-id libraryName=package-name query='what you need'
   ```

2. Then use the returned libraryId to query documentation:
   ```bash
   uxc https://mcp.context7.com/mcp query-docs libraryId=/org/project query='your question'
   ```

## Common Use Cases

### Find React hooks documentation

```bash
uxc https://mcp.context7.com/mcp resolve-library-id libraryName=react query='useState hook'
```

### Query specific API

```bash
uxc https://mcp.context7.com/mcp query-docs '{"libraryId":"/reactjs/react.dev","query":"how to use useEffect"}'
```

### Find Node.js fs module docs

```bash
uxc https://mcp.context7.com/mcp resolve-library-id libraryName=node query='file system'
```

## Output Handling

Parse the response:

```bash
# Extract the answer text
uxc https://mcp.context7.com/mcp query-docs libraryId=/reactjs/react.dev query=useState | jq -r '.data.content[].text'
```

## Fallback: Explicit JSON Flag

```bash
uxc https://mcp.context7.com/mcp query-docs --input-json '{"libraryId":"/reactjs/react.dev","query":"useMemo"}'
```

## Limitations

- Must resolve library first before querying
- Some libraries may have multiple matches - choose the best one
- Context7 provides up-to-date docs, but coverage varies by library
