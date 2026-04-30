# MVP Implementation Plan

This folder breaks the V1 local workflow into implementation chunks that can be picked up one by one.

MVP scope:

- Build a local Rust CLI named `sw-impact`.
- Build a SQLite index from the latest plugin corpus.
- Extract real Shopware surface usages from PHP, Twig, XML, JSON, YAML, TOML, JS/TS, and Vue script/template blocks.
- Check a local Shopware worktree against `upstream/trunk`.
- Include committed changes, uncommitted tracked changes, and untracked files.
- Report affected plugins with file/line evidence.
- Treat the report as impact evidence, not breakage proof.

## Implementation Order

1. [Project Foundation](01-project-foundation.md)
2. [Corpus Walk And Prefilter](02-corpus-walk-prefilter.md)
3. [Fact Model](03-fact-model.md)
4. [PHP Extractor](04-php-extractor.md)
5. [Twig And Config Extractors](05-twig-config-extractors.md)
6. [SQLite Index And Query](06-sqlite-index-query.md)
7. [Shopware Change Detection](07-shopware-change-detection.md)
8. [Impact Report](08-impact-report.md)
9. [Administration JS/TS Extractor](09-admin-js-ts-extractor.md)
10. [Signature Classification And Hardening](10-signature-hardening.md)

## MVP Done

The MVP is done when:

- `sw-impact index --plugins ... --out ...` builds a SQLite index from the plugin corpus.
- `sw-impact query --index ... <surface-key>` returns impacted plugins with evidence.
- `sw-impact check --shopware ... --base upstream/trunk --index ...` reports impacted plugins for local Shopware changes.
- PHP, Twig, config, and Administration extraction are backed by fixtures.
- High and medium confidence hits are included by default.
- Low-confidence hits are opt-in.
- The report includes changed surface, change type, confidence, affected plugin count, usage count, and file/line evidence.
- Known limitations are documented in CLI help or README text.

## Cross-Cutting Rules

- Keep each chunk releasable and covered by fixtures where possible.
- Prefer exact structured parsing over text scanning when a maintained parser is available.
- Avoid whole-program analysis in MVP.
- Keep low-confidence behavior explicit.
- Measure performance on corpus samples before optimizing heavily.
- Preserve a clean internal split between extraction, indexing, checking, and reporting.
