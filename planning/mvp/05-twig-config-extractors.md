# Chunk 05: Twig And Config Extractors

## Goal

Implement real extraction for Twig and common Shopware/Symfony config files.

## Scope

- Twig scanner.
- XML extractor.
- JSON extractor.
- YAML extractor.
- TOML extractor.
- Usage facts for plugin indexing.
- Definition facts where Shopware files define routes, services, blocks, templates, app metadata, or theme metadata.

## Twig Extraction

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

Suggested surface keys:

```text
twig:template:@Storefront/storefront/page/product-detail/index.html.twig
twig:block:storefront_page_product_detail_buy
route:name:frontend.detail.page
```

## Config Extraction

Use native parsers:

- `quick-xml`
- `serde_json`
- `serde_yaml` or another maintained YAML parser
- `toml`

Extract:

- Symfony service IDs.
- Service decorations.
- Tags.
- Event listener/subscriber config.
- Routes.
- Composer Shopware constraints.
- App manifest webhooks and permissions.
- Theme metadata.

## Implementation Notes

- Twig scanner can be token/regex-like as long as fixtures cover common syntax.
- XML should be parsed structurally, not with ad hoc string splitting.
- JSON/YAML/TOML should use serde-style parsing.
- Do not fail the whole index on one malformed config file; record a warning and continue.

## Fixtures

Add fixtures for:

- Twig block override.
- Twig template extension.
- Twig include.
- Twig route generation.
- Symfony `services.xml`.
- Symfony route XML/YAML.
- `composer.json` Shopware constraints.
- `manifest.xml`.
- `theme.json`.

## Acceptance

- Fixtures emit expected surface keys and evidence lines.
- Malformed config files are reported but do not abort indexing by default.
- Twig blocks and templates are high confidence.
- Route/service/entity strings are high or medium confidence based on structured context.

## Out Of Scope

- Full Twig AST parser.
- Deep CSS/SCSS selector analysis.
- Complete Symfony container compilation.
