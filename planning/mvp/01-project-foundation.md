# Chunk 01: Project Foundation

## Goal

Create the Rust CLI foundation for `sw-impact` so later chunks have stable command wiring, error handling, logging, and test structure.

## Scope

- Create Rust workspace.
- Add `sw-impact` binary.
- Add command shells:
  - `index`
  - `check`
  - `query`
- Add shared configuration structs.
- Add basic logging/progress output.
- Add integration test scaffolding.
- Add developer README with build/test commands.

## Suggested Crates

- `clap` for CLI parsing.
- `anyhow` or `miette` for application errors.
- `thiserror` for library errors if needed.
- `tracing` and `tracing-subscriber` for logging.
- `assert_cmd` and `predicates` for CLI tests.
- `tempfile` for fixtures and temporary indexes.

## Command Shape

```bash
sw-impact index \
  --plugins /path/to/plugins \
  --out .sw-impact/plugins.sqlite

sw-impact check \
  --shopware /path/to/shopware \
  --base upstream/trunk \
  --index .sw-impact/plugins.sqlite

sw-impact query \
  --index .sw-impact/plugins.sqlite \
  "php:class:Shopware\Core\Checkout\Cart\CartService"
```

## Implementation Notes

- The commands can initially return "not implemented" after validating arguments.
- Keep command modules small:
  - `cli`
  - `commands::index`
  - `commands::check`
  - `commands::query`
- Keep reusable logic outside command modules from the start.
- Prefer absolute paths in normalized runtime config, but preserve display paths for output.

## Acceptance

- `cargo test` passes.
- `sw-impact --help` lists all commands.
- Each command validates required arguments.
- Each command has at least one integration test for argument parsing.
- README documents how to build and run the binary locally.

## Out Of Scope

- File walking.
- Parsing.
- SQLite.
- Git change detection.
- Final report formatting.
