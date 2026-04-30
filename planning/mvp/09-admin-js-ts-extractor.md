# Chunk 09: Administration JS/TS Extractor

## Goal

Implement real Administration JavaScript, TypeScript, and Vue extraction using Oxc for script parsing.

## Scope

- Parse `.js` and `.ts` with Oxc.
- Split `.vue` files into blocks.
- Parse `<script>` and `<script setup>` with Oxc.
- Scan Vue templates for relevant Shopware component, route, and entity references.
- Emit usage facts for plugin indexing.
- Emit definition facts for Shopware Administration source where possible.

## Extract

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

## Surface Examples

```text
admin:component:sw-product-detail
admin:route:sw.product.detail
admin:state-store:swProductDetail
dal:entity:product
```

## Implementation Notes

- Do not implement a general JavaScript type system.
- Exact literal arguments are high confidence.
- Simple string constants can be medium confidence if locally obvious.
- Dynamic expressions are low confidence or unsupported in MVP.
- Keep Vue block splitting simple and fixture-driven.

## Fixtures

Add fixtures for:

- Component override.
- Component extend.
- Component register.
- Module register.
- Route name.
- Entity repository creation.
- Service access.
- State access.
- Vue script block.
- Vue script setup block.
- Vue template component usage.

## Acceptance

- Fixture tests emit expected Admin and DAL surface keys.
- Exact literal component/route/entity references include line evidence.
- Vue script blocks are parsed.
- Vue template scan emits useful component references.

## Out Of Scope

- Full Vue compiler integration.
- Deep JS type inference.
- Minified/generated bundle analysis.
