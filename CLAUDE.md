# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

The canonical agent guide for this repository is `AGENTS.md`, imported below. Keep
repository rules there, not here, so all coding agents share one source of truth.

@AGENTS.md

## Claude Code notes

- Claude Code auto-loads only this file and the root `AGENTS.md`. Nested guides
  (`src/web-ui/AGENTS.md`, `src/apps/desktop/AGENTS.md`, `src/crates/<layer>/AGENTS.md`,
  per-crate `AGENTS.md`, etc.) are **not** loaded automatically. Before editing or
  verifying in a directory, read the nearest `AGENTS.md` yourself; it holds the
  focused build/test commands and local ownership rules, and it wins over the root guide.
- Node >= 22.12 and pnpm are required; the Plugin Host (`src/apps/extension-host`) uses bun.

## Focused test command shapes

Use the exact commands listed in the nearest `AGENTS.md`; these are the general forms:

```bash
# Rust: one package, optionally one test filter
cargo test --locked -p <crate-name> --lib <module_or_test_filter>
cargo check -p <crate-name>

# Web UI (Vitest): one or more test files
pnpm --dir src/web-ui run test:run <path/to/file.test.tsx>
pnpm run type-check:web
pnpm run lint:web

# Repository scripts (node:test)
node --test scripts/<script>.test.mjs
```

Rust lint: `pnpm run lint:rs` (workspace minus desktop) or `pnpm run lint:rs:desktop`.
