# Shopware Change Impact CLI

Status: ready for planning.

## Planning Objective

Build a simple, fast local CLI that answers:

> If this Shopware change lands, which known third-party extensions touch the changed surface, and where?

The V1 product is not a breakage proof. It is a blast-radius report with evidence.

The first implementation should optimize for:

- Local execution.
- Fast lookup.
- Traceable file/line evidence.
- Useful approximation over deep semantic certainty.
- A small implementation that can later move into CI.

Target corpus today:

- 3204 latest plugin versions.
- 6 GB source code.
- 438686 files.
- 48881497 lines.

## V1 Decisions

These decisions are fixed for planning unless implementation proves one wrong:

- Use a single Rust CLI named `sw-impact`.
- Start local-first. CI behavior comes after the local command is useful.
- Index only latest plugin versions in V1.
- Store the plugin index in SQLite.
- Model storage as an inverted index from Shopware surface key to plugin evidence.
- Use high and medium confidence matches by default.
- Make low-confidence matches opt-in at index and report time.
- Store evidence snippets by default, with `--no-snippets` for smaller/faster indexes.
- Skip files larger than `2 MiB` by default, with an override.
- Exclude generated/dependency directories by default.
- Use `tree-sitter-php` for PHP.
- Use Oxc for JS/TS/Admin.
- Use a custom scanner for Twig.
- Use native Rust parsers for XML, JSON, YAML, and TOML.
- Local reports should exit successfully by default. Failure thresholds belong to later CI mode.

## Non-Goals For V1

- Do not prove that a plugin definitely breaks.
- Do not run plugin test suites.
- Do not perform whole-program PHP type inference.
- Do not build a server-backed indexing platform.
- Do not require Postgres, OpenSearch, or other infrastructure.
- Do not index historical plugin versions.
- Do not build customer, install, or revenue weighting.

The report should say:

```text
Known extensions touch this changed Shopware surface.
Here are the files and lines that prove it.
```

## Core CLI

### Build The Plugin Index

```bash
sw-impact index \
  --plugins /path/to/plugin-marketplace \
  --out .sw-impact/plugins.sqlite
```

Required behavior:

- Walk the plugin corpus.
- Apply default excludes.
- Prefilter files cheaply.
- Extract Shopware surface usages.
- Write a new SQLite index file.
- Store plugin, file, surface, impact, and evidence records.

Useful options:

```bash
--threads auto
--force
--no-snippets
--include-low-confidence
--plugin-manifest "**/composer.json"
--exclude vendor,node_modules,dist,build,var,cache
--max-file-size 2MiB
```

### Check A Local Shopware Branch

```bash
sw-impact check \
  --shopware /path/to/shopware \
  --base origin/trunk \
  --index .sw-impact/plugins.sqlite
```

Required behavior:

1. Resolve the merge base with `origin/trunk`.
2. List changed Shopware files.
3. Include committed changes, uncommitted tracked changes, and untracked files.
4. Extract Shopware surfaces from base and current content.
5. Diff the extracted surfaces.
6. Classify removed, renamed, and signature-changed surfaces where possible.
7. Query impacted plugins from the index.
8. Print a terminal report with affected plugin counts and file/line evidence.

### Query A Single Surface

```bash
sw-impact query \
  --index .sw-impact/plugins.sqlite \
  "php:method:Shopware\Core\Checkout\Cart\CartService::recalculate"
```

Required behavior:

- Resolve an exact surface key.
- Print impacted plugins.
- Include usage count, confidence, usage kind, and evidence locations.

## Architecture

The CLI has two data flows.

### Index Flow

```text
plugin files
  -> gitignore-aware walker
  -> cheap prefilter
  -> language extractor
  -> normalized facts
  -> aggregate in memory
  -> SQLite inverted index
```

### Check Flow

```text
Shopware base/current changed files
  -> language extractor
  -> base/current surface facts
  -> surface diff
  -> SQLite lookup
  -> impact report
```

The same extractors should be usable for plugin indexing and Shopware change extraction. The output side differs:

- Plugin indexing records usages of Shopware surfaces.
- Shopware checking records definitions and changed surfaces.

## Storage

### V1 Storage: SQLite

SQLite is the V1 storage choice because it is:

- One file.
- Easy to inspect locally.
- Easy to ship into CI later.
- Fast enough for exact key lookups.
- Good for traceability evidence.

The schema should stay focused on lookup, not become a general source-code database.

Schema sketch:

```sql
create table plugin (
    id integer primary key,
    name text not null,
    version text,
    path text not null
);

create table file (
    id integer primary key,
    plugin_id integer not null,
    path text not null,
    hash blob
);

create table surface (
    id integer primary key,
    key text not null unique,
    kind text not null
);

create table impact (
    surface_id integer not null,
    plugin_id integer not null,
    usage_count integer not null,
    confidence integer not null,
    primary key (surface_id, plugin_id)
);

create table evidence (
    surface_id integer not null,
    plugin_id integer not null,
    file_id integer not null,
    line integer not null,
    column integer,
    usage_kind text not null,
    snippet text,
    confidence integer not null
);

create index evidence_surface_plugin on evidence(surface_id, plugin_id);
create index impact_surface on impact(surface_id);
```

Build-time rules:

- Create a new SQLite file for each full rebuild.
- Use one write transaction.
- Batch inserts.
- Create indexes after bulk inserts.
- Store snippets unless `--no-snippets` is set.
- Store high and medium confidence by default.
- Store low confidence only when `--include-low-confidence` is set.

### Later Storage Option

If SQLite build time or file size becomes the bottleneck, replace the storage writer with a compact read-only format:

```text
surfaces.fst        surface key -> numeric surface id
postings.bin.zst    surface id -> compressed plugin id list
evidence.bin.zst    surface/plugin -> file/line records
plugins.bin.zst
files.bin.zst
```

Possible data structures:

- `fst` for surface key lookup.
- Sorted `u16` arrays for plugin postings while plugin count stays below 65536.
- Roaring bitmaps if plugin count grows or set operations become important.
- `zstd` compression for evidence.

This is deferred. SQLite is better for first delivery because it is inspectable.

## Surface Keys

Represent every touched Shopware thing as a stable string key.

Examples:

```text
php:class:Shopware\Core\Checkout\Cart\CartService
php:method:Shopware\Core\Checkout\Cart\CartService::recalculate
php:const:Shopware\Core\Framework\Context::SYSTEM_SCOPE
service:id:cart.processor
event:class:Shopware\Core\Checkout\Cart\Event\CheckoutOrderPlacedEvent
event:name:checkout.order.placed
route:name:store-api.checkout.cart
api:store-api:POST:/store-api/checkout/cart
dal:entity:product
dal:field:product.price
twig:template:@Storefront/storefront/page/product-detail/index.html.twig
twig:block:storefront_page_product_detail_buy
admin:component:sw-product-detail
admin:route:sw.product.detail
admin:state-store:swProductDetail
storefront-js-plugin:ListingPlugin
feature-flag:FEATURE_NEXT_12345
```

Do not index only public API. Index internal usage too, but classify it separately in the report when known.

## Extractors

### File Discovery

Use Rust crates:

- `ignore` for gitignore-aware walking.
- `rayon` for parallel processing.
- `memmap2` or buffered reads depending on benchmark results.

Default excludes:

```text
vendor/
node_modules/
dist/
build/
var/
cache/
.git/
```

Default file size behavior:

- Skip files larger than `2 MiB`.
- Do not skip known manifests solely by size without warning.
- Report skipped file counts in verbose mode.

### Prefilter

Use `aho-corasick` and `memchr` to cheaply decide whether a file is worth parsing.

Candidate anchors:

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

The prefilter must be conservative. False positives are acceptable. False negatives should be rare.

### PHP

Use `tree-sitter-php`.

Extract high-confidence facts:

- Fully qualified class references.
- `use Shopware\...` imports and aliases.
- Class inheritance.
- Implemented interfaces.
- Used traits.
- Attributes.
- Type hints.
- Return types.
- Static calls.
- Constants and enum cases.
- `new` expressions.
- Event subscriber mappings.

Extract medium-confidence facts:

- Method calls where the receiver type can be locally inferred.
- Service IDs in strings.
- DAL entity names in strings.
- Repository service IDs.
- Route names in attributes or strings.

Do not attempt whole-program type inference in V1. A per-file namespace/import map is enough for useful first results.

### Twig

Start with a custom scanner.

Extract:

- `sw_extends`.
- `extends`.
- `include`.
- `sw_include`.
- `{% block ... %}`.
- `path(...)`.
- `seoUrl(...)`.
- Twig functions, filters, and tests.
- Template names such as `@Storefront/...`.

Twig is high value because block and template changes are common extension break points.

### XML, JSON, YAML, TOML

Use native Rust parsers:

- `quick-xml`.
- `serde_json`.
- `serde_yaml` or another maintained YAML parser.
- `toml`.

Extract:

- Symfony service IDs.
- Service decorations.
- Tags.
- Event listener/subscriber config.
- Routes.
- Composer Shopware constraints.
- App manifest webhooks and permissions.
- Theme metadata.

### JavaScript, TypeScript, Administration

Use Oxc.

Extract:

- Imports from Shopware Administration source.
- `Shopware.Component.override`.
- `Shopware.Component.extend`.
- `Shopware.Component.register`.
- `Shopware.Module.register`.
- `Shopware.Service`.
- `Shopware.State`.
- `repositoryFactory.create(...)`.
- Route names.
- Entity names.
- Component names.

For `.vue` files:

1. Split the SFC into blocks cheaply.
2. Parse `<script>` and `<script setup>` with Oxc.
3. Scan template blocks for Shopware component names and route/entity strings.

### CSS and SCSS

Defer deep CSS handling.

CSS selector impact is noisy. V1 can collect obvious selector strings from JS/Twig only if they are already encountered by those extractors. A real CSS parser such as Lightning CSS can be evaluated later.

## Shopware Change Extraction

For `sw-impact check`, compare base and current versions of changed files:

```text
base Shopware file surfaces
current Shopware file surfaces
diff
changed or removed surface keys
```

High-value detected changes:

- Removed class/interface/trait/enum.
- Removed method/property/constant/enum case.
- Method signature changed.
- Constructor signature changed.
- Visibility changed.
- Added required parameter.
- Removed service ID or alias.
- Removed event class or event name.
- Removed route name.
- Removed DAL entity or field.
- Removed Twig template or block.
- Removed Administration component/route/state API.
- Removed Storefront JS plugin.

Initial change classification should focus on removed and renamed surfaces. Signature classification comes after exact surface removal reporting works.

## Confidence

Confidence keeps the report honest.

High confidence:

- Resolved PHP FQCN.
- Class inheritance/interface/trait reference.
- Exact constant/static call.
- Exact service ID.
- Exact event class.
- Exact route name.
- Exact Twig block override.
- Exact Admin component override.

Medium confidence:

- String references to known Shopware IDs.
- Locally inferred PHP method call.
- Entity or repository name strings.
- JS property paths under `Shopware`.

Low confidence:

- Dynamic class names.
- Reflection.
- Container access with dynamic IDs.
- Concatenated route/template/service names.
- Plain text hits.

Default behavior:

- Index high and medium confidence.
- Report high and medium confidence.
- Include low confidence only with explicit opt-in.

## Report

Example output:

```text
Shopware Impact Report
Base: origin/trunk
Compared: working tree

High confidence impact

php:method:Shopware\Core\Checkout\Cart\CartService::recalculate
change: signature changed
classification: public
affected plugins: 18
usages: 41

  PluginA  src/Subscriber/CartSubscriber.php:42  method-call
  PluginB  src/Service/Foo.php:88                static-call
  PluginC  src/Checkout/Bar.php:21               type-hint

Twig impact

twig:block:storefront_page_product_detail_buy
change: removed
classification: storefront-extension-point
affected plugins: 73
usages: 96

  PluginD  src/Resources/views/storefront/page/product-detail/index.html.twig:7  block-override
```

The report must include:

- Surface key.
- Change type.
- Public/internal/deprecated classification when known.
- Affected plugin count.
- Usage count.
- Evidence file and line.
- Confidence.

Later report improvements:

- JSON output.
- CI thresholds.
- Installation/customer/revenue weighting from extension-store data.
- Links to changed Shopware files.

## Performance Targets

Initial targets on the current corpus:

- Full plugin index build: seconds to low tens of seconds on a modern developer machine.
- Local branch check: below 2 seconds for normal PR-sized changes.
- Query one surface: below 100 ms.

Performance techniques:

- Parallel file walking and parsing.
- Conservative prefilter before parsing.
- Parse only supported file types.
- Skip large generated files by size threshold unless explicitly included.
- Aggregate facts in memory before writing.
- Use numeric IDs internally.
- Create SQLite indexes after bulk inserts.
- Keep evidence snippets optional.

## Planning Slices

### Slice 1: CLI Skeleton And Corpus Walk

Deliverables:

- Rust workspace and `sw-impact` binary.
- `index`, `check`, and `query` command shells.
- File walker with default excludes.
- Thread and file-size options.
- Basic progress/log output.

Acceptance:

- `sw-impact index --plugins ... --out ...` walks the target corpus without indexing facts yet.
- Skipped file counts are visible in verbose mode.

### Slice 2: PHP, Twig, And Config Fact Extraction

Deliverables:

- Shared fact model.
- PHP extractor with namespace/import resolution.
- Twig scanner.
- XML/JSON/YAML/TOML extractors.
- Confidence and usage kind assignment.

Acceptance:

- Extractors emit normalized surface keys with file/line evidence.
- Unit fixtures cover representative PHP, Twig, service XML, route config, composer, and manifest examples.

### Slice 3: SQLite Index And Query

Deliverables:

- SQLite writer.
- Aggregation by surface/plugin.
- Query path for exact surface keys.
- `--no-snippets` and `--include-low-confidence`.

Acceptance:

- A full index builds into one SQLite file.
- `sw-impact query` returns impacted plugins and evidence for known fixture surfaces.
- Bulk insert performance is measured on a meaningful corpus sample.

### Slice 4: Shopware Branch Check

Deliverables:

- Git merge-base handling.
- Changed file discovery.
- Uncommitted tracked and untracked file handling.
- Base/current extraction.
- Removed/renamed surface diff.
- Index lookup for changed surfaces.

Acceptance:

- `sw-impact check` reports impacted plugins for a local Shopware branch.
- The command includes working tree changes.
- Normal PR-sized checks complete below the target runtime.

### Slice 5: Human Report

Deliverables:

- Grouped terminal report.
- Surface-level summaries.
- Plugin evidence rows.
- Confidence display.
- Public/internal/deprecated classification when known.

Acceptance:

- Report is understandable without opening SQLite.
- Report differentiates high and medium confidence.
- Report makes clear that impact is a touch signal, not a breakage proof.

### Slice 6: Administration JS/TS

Deliverables:

- Oxc-based JS/TS extractor.
- `.vue` script block support.
- Template scan for relevant component, route, and entity references.

Acceptance:

- Extractor covers common Administration extension patterns.
- Fixtures include component override/extend/register, module registration, route names, entity names, and repository creation.

### Slice 7: Signature Classification And Hardening

Deliverables:

- Method and constructor signature comparison.
- Visibility and required-parameter classification.
- Better edge-case handling.
- Performance pass on full corpus.

Acceptance:

- Signature changes are classified separately from removal.
- Full corpus build and local check meet target timings or document measured bottlenecks.

## Definition Of Done For V1

V1 is done when:

- A developer can build an index from the current plugin corpus.
- A developer can run `check` against a local Shopware worktree.
- The report includes changed Shopware surfaces, affected plugin counts, and file/line evidence.
- High and medium confidence results work by default.
- Low confidence is explicitly opt-in.
- Runtime is close enough to targets to be useful locally.
- Fixtures cover each implemented extractor.
- Known limitations are documented in CLI help or README text.

## Planning Risks

- False negatives from prefiltering.
  Mitigation: keep anchors conservative and add fixture coverage for known extension patterns.

- Noisy string-based matches.
  Mitigation: confidence levels and default hiding of low-confidence results.

- PHP method-call resolution can expand in scope.
  Mitigation: limit V1 to per-file namespace/import resolution plus simple local inference.

- Full corpus indexing may exceed target runtime.
  Mitigation: measure early after slice 1 and slice 3, then tune walking, batching, and snippets.

- Surface classification may need Shopware-specific rules.
  Mitigation: start with explicit known categories and let unknown surfaces still report impact.

## Remaining Open Questions

These are not blockers for implementation planning, but they need owners before V1 completion:

- Which exact plugin metadata source provides plugin name and version?
- Which Shopware rules classify a surface as public, internal, deprecated, or extension point?
- Should JSON output be included in V1 or be the first CI follow-up?
- What CI threshold should eventually produce a non-zero exit code?
- How should renamed surfaces be detected beyond simple remove/add heuristics?

## Recommended First Planning Tickets

1. Create Rust CLI workspace and command structure.
2. Implement corpus walker with excludes, file-size threshold, and progress output.
3. Define normalized fact, surface, confidence, and evidence data types.
4. Implement PHP extractor fixtures and parser integration.
5. Implement Twig scanner fixtures.
6. Implement XML/JSON/YAML/TOML extractor fixtures.
7. Implement SQLite schema, bulk writer, and exact query.
8. Implement Shopware changed-file discovery including uncommitted changes.
9. Implement base/current surface diff for removals.
10. Implement first terminal report.

## References

- Shopware backward compatibility guideline: https://developer.shopware.com/docs/resources/guidelines/code/backward-compatibility.html
- Tree-sitter: https://tree-sitter.github.io/tree-sitter/
- Oxc parser: https://oxc.rs/docs/learn/architecture/parser.html
- SQLite appropriate uses: https://www.sqlite.org/whentouse.html
- Roaring bitmap Rust crate: https://docs.rs/roaring/latest/roaring/
