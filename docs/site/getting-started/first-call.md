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

- Use key/value arguments for simple calls.
- Use positional JSON for nested objects.
- Keep `--text` for human-readable output; JSON is the default contract.
