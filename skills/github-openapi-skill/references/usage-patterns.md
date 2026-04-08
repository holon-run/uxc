# GitHub REST API Usage Patterns

## One-time setup

```bash
uxc auth credential import github --from gh
uxc auth binding match https://api.github.com/repos/holon-run/uxc
uxc link github-openapi-cli https://api.github.com --schema-url https://raw.githubusercontent.com/github/rest-api-description/main/descriptions/api.github.com/api.github.com.json
```

## Read current user

```bash
github-openapi-cli get:/user
```

## Read a repository

```bash
github-openapi-cli get:/repos/{owner}/{repo} owner=holon-run repo=uxc
```

## List issues

```bash
github-openapi-cli get:/repos/{owner}/{repo}/issues owner=holon-run repo=uxc state=open per_page=10
```

## List repository events

```bash
github-openapi-cli get:/repos/{owner}/{repo}/events owner=holon-run repo=uxc per_page=10
```

## Inspect a specific operation before calling it

```bash
github-openapi-cli get:/repos/{owner}/{repo}/pulls -h
```
