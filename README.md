# sw-impact

Local Shopware change-impact CLI.

`sw-impact` builds a local index of third-party plugin usages and checks a local
Shopware worktree for changed surfaces that known plugins touch.

## Build

```bash
cargo build
```

## Usage

Build a plugin index:

```bash
cargo run -- index \
  --plugins /path/to/plugin-marketplace \
  --out .sw-impact/plugins.sqlite
```

The command prints a completion line with runtime, indexed plugin count,
candidate file count, and stored evidence count.

Check a local Shopware worktree:

```bash
cargo run -- check \
  --shopware /path/to/shopware \
  --base upstream/trunk \
  --index .sw-impact/plugins.sqlite
```

Query one surface:

```bash
cargo run -- query \
  --index .sw-impact/plugins.sqlite \
  "php:class:Shopware\Core\Checkout\Cart\CartService"
```

## Implemented MVP Scope

- Plugin corpus walking with default excludes and conservative prefiltering.
- SQLite index creation with plugin, file, surface, impact, evidence, and metadata tables.
- Exact surface queries with file/line evidence.
- GitHub source links for store-plugin-mirror paths.
- Local Shopware worktree checks including committed, staged, unstaged, and untracked files.
- PHP extraction for imports, FQCNs, static calls, type hints, service strings, route strings, DAL entities, and definitions.
- Twig extraction for templates, blocks, includes, `path()`, and `seoUrl()`.
- XML/JSON/YAML/TOML extraction for services, routes, events, entities, composer constraints, and app/theme metadata.
- Administration JS/TS/Vue literal extraction for components, modules, services, state stores, routes, and repository entities.

## Current Limits

- The report is touch evidence, not proof of breakage.
- PHP analysis is per-file and does not perform whole-program type inference.
- Administration extraction is currently literal/scanner-based; deeper Oxc AST extraction is still a follow-up.
- PHP extraction currently combines tree-sitter parsing with targeted scanners; moving more extraction onto AST queries is a follow-up hardening task.
- Low-confidence facts are excluded unless `--include-low-confidence` is used.
