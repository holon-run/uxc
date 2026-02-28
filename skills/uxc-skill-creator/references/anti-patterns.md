# Anti-Patterns

## 1. Probe-first auth by remote call

Avoid adding extra remote probe calls just to test auth readiness.
Use local binding checks first, then validate through the first real operation call.

## 2. Dynamic link renaming at runtime

Do not teach runtime logic to append suffixes or auto-rename command names.
Link naming is a skill author decision at design time.

## 3. Overweight wrapper skill

Do not duplicate generic OAuth/error docs in each provider skill.
Keep wrappers thin and reference `skills/uxc` for shared behavior.

## 4. Ambiguous invocation style

Avoid mixed, obsolete, or conflicting argument styles in examples.
Prefer `key=value`, and include bare JSON positional only where it adds clarity.

## 5. Write-before-read flows

Do not default to write operations.
Read current state first, then require explicit user confirmation before destructive or high-impact writes.

## 6. Host-only assumptions

Do not infer protocol/path/auth only from host shape.
Always verify with official docs search and `uxc` probing before publishing wrapper commands.
