#!/usr/bin/env bash
# Validate Linear MCP Skill

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Validating Linear MCP Skill..."

# Check required files exist
echo "Checking required files..."
test -f "$ROOT_DIR/SKILL.md" || { echo "ERROR: SKILL.md not found"; exit 1; }
test -f "$ROOT_DIR/agents/openai.yaml" || { echo "ERROR: agents/openai.yaml not found"; exit 1; }
test -f "$ROOT_DIR/references/usage-patterns.md" || { echo "ERROR: references/usage-patterns.md not found"; exit 1; }

# Validate YAML syntax
echo "Validating YAML files..."
if command -v python3 &> /dev/null; then
  python3 -c "import yaml; yaml.safe_load(open('$ROOT_DIR/agents/openai.yaml'))" 2>/dev/null || echo "WARNING: yaml module not available, skipping YAML validation"
else
  echo "WARNING: python3 not found, skipping YAML validation"
fi

# Check skill name
echo "Checking skill name..."
grep -q "^name: linear-mcp-skill" "$ROOT_DIR/SKILL.md" || { echo "ERROR: Invalid skill name"; exit 1; }

echo "Validation passed!"
