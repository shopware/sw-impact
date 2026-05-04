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

Before you can do that you need to clone the [store-plugin-mirror](https://github.com/shopware/store-plugin-mirror) repo:
```bash
git clone --depth 1 git@github.com:shopware/store-plugin-mirror.git
```

Then you can build the index based on that (this takes a while, just wait - 5 min. on my machine):

```bash
./target/release/sw-impact index \
  --plugins ../store-plugin-mirror/plugins/shopware6 \
  --out index.sqlite \
  --force
```

The command prints a completion line with runtime, indexed plugin count,
candidate file count, and stored evidence count. The index stores plugin folder
names, not absolute host paths.

Check a local Shopware worktree:

```bash
./target/release/sw-impact check \
  --shopware /path/to/shopware \
  --index index.sqlite
```

`check` defaults to `--base origin/trunk`, `--max-surfaces 5`, and
`--max-evidence-per-surface 10`.

Query one surface:

```bash
./target/release/sw-impact query \
  --index index.sqlite \
  "php:class:Shopware\Core\Checkout\Cart\CartService"
```

Query surfaces with wildcards:

```bash
./target/release/sw-impact query \
  --index index.sqlite \
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

## Example output

After removing the file content of `src/Storefront/Resources/views/storefront/page/content/detail.html.twig`,
which removes two twig blocks, you get this output:

```txt
Shopware Impact Report

High confidence impact

--------------------------------------------------------------------------------
twig:block:page_content_section
  surface kind:     twig-block
  change:           removed
  shopware source:  src/Storefront/Resources/views/storefront/page/content/detail.html.twig:41:22 (base)
  confidence:       high
  affected plugins: 47
  usages:           52
  shopware snippet: {% block page_content_section %}
--------------------------------------------------------------------------------
  1. acris/cms@v9.17.2
     File:       src/Resources/views/storefront/page/content/detail.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  2. acris/cms-layout@v8.3.22
     File:       src/Resources/views/storefront/page/content/detail.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  3. acris/cms-product-listing@v6.0.4
     File:       src/Resources/views/storefront/page/content/detail.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  4. acris/personal-contact@v5.2.2
     File:       src/Resources/views/storefront/page/account/personalContacts/cms.html.twig:40
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  5. arvenio/automatic-links@7.0.0
     File:       src/Resources/views/storefront/page/content/detail.html.twig:5
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  6. avalex/avalex-shopware@3.0.4
     File:       src/Resources/views/storefront/page/content/detail.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  7. bc/brand-crock-sports-zone@4.0.1
     File:       src/Resources/views/storefront/page/content/detail.html.twig:5
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  8. bwtsds/cms-partials@1.2.2
     File:       src/Resources/views/storefront/element/cms-element-bwt-partial.html.twig:20
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  9. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/content.html.twig:22
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  10. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/detail.html.twig:22
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_section %}
  ... (10 of 52 shown)

--------------------------------------------------------------------------------
twig:block:page_content_sections_inner
  surface kind:     twig-block
  change:           removed
  shopware source:  src/Storefront/Resources/views/storefront/page/content/detail.html.twig:4:10 (base)
  confidence:       high
  affected plugins: 23
  usages:           27
  shopware snippet: {% block page_content_sections_inner %}
--------------------------------------------------------------------------------
  1. acris/personal-contact@v5.2.2
     File:       src/Resources/views/storefront/page/account/personalContacts/cms.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  2. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/content.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  3. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/detail.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  4. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/index.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  5. cbax/modul-lexicon@5.0.7
     File:       src/Resources/views/storefront/cbax-lexicon/listing.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  6. cbax/modul-manufacturers@5.0.8
     File:       src/Resources/views/storefront/cbax-manufacturer/detail.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  7. cbax/modul-manufacturers@5.0.8
     File:       src/Resources/views/storefront/cbax-manufacturer/index.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  8. cogi/cms-scroll-fx@1.0.7
     File:       src/Resources/views/storefront/page/content/detail.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  9. cogi/cogi-cart-experience@2.1.0
     File:       src/Resources/views/storefront/page/content/cart-experience-content.html.twig:3
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  10. cogi/exit-intent-popup@4.0.0
     File:       src/Resources/views/storefront/utilities/exit-intent-popup-cms.html.twig:7
     Usage:      twig.block.usage
     Confidence: high
     Source:     GitHub ↗
     Snippet:    {% block page_content_sections_inner %}
  ... (10 of 27 shown)

Known extensions touch these changed Shopware surfaces. This is touch evidence, not proof of breakage.

--------------------------------------------------------------------------------
Summary
Base:                 origin/trunk
Compared:             working tree
Runtime:              8.236s
Changed surfaces:     2
Impacted surfaces:    2
Surface blocks shown: 2
Evidence rows shown:  20
Affected plugins:     53 / 3205 (1.7%)
--------------------------------------------------------------------------------
```

Note: The `GitHub ↗` links are clickable in your CLI terminal, most likely by holding down CMD / CTRL

