# Chunk 02: Corpus Walk And Prefilter

## Goal

Implement fast plugin corpus discovery with default excludes, file-size limits, and a conservative prefilter before real parsing.

## Scope

- Walk plugin corpus roots.
- Detect plugin roots from `composer.json` by default.
- Apply default excludes:
  - `vendor/`
  - `node_modules/`
  - `dist/`
  - `build/`
  - `var/`
  - `cache/`
  - `.git/`
- Add `--exclude`.
- Add `--plugin-manifest`.
- Add `--max-file-size`, default `2MiB`.
- Add `--threads`, default `auto`.
- Add prefilter anchors.
- Emit candidate files for extractors.
- Report skipped counts in verbose mode.

## Suggested Crates

- `ignore` for gitignore-aware walking.
- `rayon` for parallel traversal/processing.
- `aho-corasick` for multi-pattern prefiltering.
- `memchr` for cheap byte checks when useful.

## Prefilter Anchors

```text
Shopware\
Shopware.
Shopware
sw_extends
@Storefront
@Administration
@Framework
services.xml
routes.xml
composer.json
manifest.xml
repositoryFactory
Component.override
Component.extend
Module.register
PluginManager
```

## Implementation Notes

- Prefer false positives over false negatives.
- File extension should also influence eligibility:
  - PHP: `.php`
  - Twig: `.twig`
  - Config: `.xml`, `.json`, `.yaml`, `.yml`, `.toml`
  - Admin: `.js`, `.ts`, `.vue`
- Known manifest files should be processed even if prefilter anchors are absent.
- Store enough metadata for later evidence:
  - plugin id/name candidate
  - plugin root
  - relative file path
  - absolute file path
  - size
  - language kind

## Acceptance

- `sw-impact index --plugins ... --out ... --verbose` walks fixture plugins and prints file counts.
- Default excludes are covered by tests.
- File-size skipping is covered by tests.
- Manifest discovery is covered by tests.
- Prefilter keeps known positive fixture files.

## Out Of Scope

- Extracting facts.
- Writing SQLite.
- Querying.
- Git change detection.
