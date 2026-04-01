# First Call

Most first calls should start from discovery, not direct execution.

## 1. Discover a host

```bash
uxc petstore3.swagger.io/api/v3 -h
```

## 2. Inspect one operation

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} -h
```

## 3. Invoke it

```bash
uxc petstore3.swagger.io/api/v3 get:/pet/{petId} petId=1
```

## Tips

- Use `key=value` for scalar fields.
- Use path-style keys for nested fields: `filter.status=active`, `items[0].id=1`.
- Use `:=` for one complex field: `filter:='{"status":"active"}'`.
- For multipart file fields, pass a local file path string: `file=/abs/path/a.pdf`.
- Use positional JSON when most fields are complex.
- Keep `--text` for human-readable output; JSON is the default contract.
