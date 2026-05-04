> [!WARNING]
> This project is a vibe-coded experiment. Treat the output as exploratory signal, not authoritative compatibility analysis.

# sw-impact

Local Shopware change-impact CLI.

`sw-impact` builds a local index of third-party plugin usages and checks a local
Shopware worktree for changed surfaces that known plugins touch.

## Build

```bash
cargo build --release
```

## Usage

Run any command with `--help` for the full option list.

Build a plugin index:

```bash
./target/release/sw-impact index \
  --plugins /path/to/plugin-marketplace \
  --out .sw-impact/plugins.sqlite \
  --force
```

The command prints a completion line with runtime, indexed plugin count,
candidate file count, and stored evidence count. The index stores plugin folder
names, not absolute host paths.

Check a local Shopware worktree:

```bash
./target/release/sw-impact check \
  --shopware /path/to/shopware \
  --base origin/trunk \
  --index .sw-impact/plugins.sqlite
```

`check` defaults to `--base origin/trunk`, `--max-surfaces 5`, and
`--max-evidence-per-surface 10`.

Query one surface:

```bash
./target/release/sw-impact query \
  --index .sw-impact/plugins.sqlite \
  "php:class:Shopware\Core\Checkout\Cart\CartService"
```

Query surfaces with wildcards:

```bash
./target/release/sw-impact query \
  --index .sw-impact/plugins.sqlite \
  "twig:block:page_content_section*"
```

`query` defaults to `--max-surfaces 5` and `--max-evidence-per-surface 10`.
`--max-evidence` is kept as an alias for `--max-evidence-per-surface`.

## Implemented MVP Scope

- Plugin corpus walking with default excludes, size limits, conservative
  prefiltering, and `src/Resources/public` bundle skipping.
- SQLite index creation with plugin, file, surface, impact, evidence, and metadata tables.
- Exact and wildcard surface queries with grouped file/line evidence.
- GitHub source links for store-plugin-mirror paths.
- Local Shopware worktree checks including committed, staged, unstaged, and untracked files.
- PHP extraction for definitions, imports, FQCNs, static calls, many instance
  calls, type hints, service strings, route strings, and DAL entities.
- Twig extraction for templates, blocks, includes/extends, `path()`, `seoUrl()`,
  and snippet translation calls.
- Snippet JSON definition extraction, with snippet usages from Twig and
  Administration `$t`, `$te`, and `$tc` calls.
- XML/JSON/YAML/TOML extraction in the plugin corpus for services, routes,
  events, entities, composer constraints, and app/theme metadata.
- Administration JS/TS/Vue scanner extraction for components, modules, services,
  state stores, routes, repository entities, snippets, and embedded Twig blocks.

## Current Limits

- The report is touch evidence, not proof of breakage.
- PHP analysis is per-file and does not perform whole-program semantic type inference.
- Administration and Twig extraction are scanner-based and only catch common literal patterns.
- Shopware XML changes are intentionally ignored by `check`.
- Low-confidence facts are excluded unless `--include-low-confidence` is used.
- Reports are truncated by default; raise `--max-surfaces` and
  `--max-evidence-per-surface` for broader output.
