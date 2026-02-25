---
name: deepwiki
description: Ask questions and read documentation about any GitHub repository using DeepWiki MCP. Use when you need to understand a codebase, find specific APIs, or get context about a repository.
metadata:
  short-description: Query GitHub repo docs via DeepWiki
---

# DeepWiki Skill

Use this skill to query GitHub repository documentation and ask questions about codebases.

## Prerequisites

- `uxc` is installed and available in `PATH`.
- Network access to `mcp.deepwiki.com/mcp`

Note: Repositories must be indexed on DeepWiki first. Visit https://deepwiki.com to index a repository.

### Install uxc

Choose one of the following methods:

**Homebrew (macOS/Linux):**
```bash
brew tap holon-run/homebrew-tap
brew install uxc
```

**Install Script (macOS/Linux, review before running):**
```bash
curl -fsSL https://raw.githubusercontent.com/holon-run/uxc/main/scripts/install.sh -o install-uxc.sh
less install-uxc.sh
bash install-uxc.sh
```

**Cargo:**
```bash
cargo install uxc
```

## Core Workflow

1. List available tools:
   - `uxc mcp.deepwiki.com/mcp list`

2. Ask a question about a repository:
   - `uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"owner/repo","question":"your question"}'`

3. Read wiki structure:
   - `uxc mcp.deepwiki.com/mcp read_wiki_structure --json '{"repoName":"owner/repo"}'`

4. Read wiki contents:
   - `uxc mcp.deepwiki.com/mcp read_wiki_contents --json '{"repoName":"owner/repo"}'`

## Available Tools

- **ask_question**: Ask any question about a GitHub repository and get an AI-powered response
- **read_wiki_structure**: Get a list of documentation topics for a repository
- **read_wiki_contents**: View documentation about a repository

## Usage Examples

### Ask about a codebase

```bash
uxc mcp.deepwiki.com/mcp ask_question --json '{"repoName":"facebook/react","question":"How does useState work?"}'
```

### Explore repository structure

```bash
uxc mcp.deepwiki.com/mcp read_wiki_structure --json '{"repoName":"facebook/react"}'
```

### Read documentation

```bash
uxc mcp.deepwiki.com/mcp read_wiki_contents --json '{"repoName":"facebook/react"}'
```

## Output Parsing

The response is an MCP JSON envelope. Extract the content from `.data.content[].text`.

## Notes

- Maximum 10 repositories per question
- Some popular repositories may already be indexed
- Responses are grounded in the actual codebase

## Reference Files

- Workflow details: `references/usage-patterns.md`
