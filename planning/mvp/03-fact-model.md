# Chunk 03: Fact Model

## Goal

Define the normalized internal model shared by extractors, index writing, Shopware change detection, and reporting.

## Scope

- Define `SurfaceKey`.
- Define `SurfaceKind`.
- Define `Fact`.
- Define `FactRole`.
- Define `Evidence`.
- Define `Confidence`.
- Define `UsageKind`.
- Define `ChangeKind`.
- Define `SurfaceClassification`.

## Model Sketch

```rust
struct Fact {
    surface: SurfaceKey,
    kind: SurfaceKind,
    role: FactRole,
    evidence: Evidence,
    confidence: Confidence,
    usage_kind: UsageKind,
}
```

Important distinction:

- Plugin indexing emits usage facts.
- Shopware checking emits definition facts.

```rust
enum FactRole {
    Usage,
    Definition,
}
```

## Surface Key Rules

Surface keys are stable strings such as:

```text
php:class:Shopware\Core\Checkout\Cart\CartService
php:method:Shopware\Core\Checkout\Cart\CartService::recalculate
service:id:cart.processor
twig:block:storefront_page_product_detail_buy
admin:component:sw-product-detail
```

Rules:

- The key format must be deterministic.
- Keys must not depend on local file paths.
- Keys should be plain strings at storage boundaries.
- Internally, parsing helpers may use typed structs before formatting.

## Confidence

```rust
enum Confidence {
    High,
    Medium,
    Low,
}
```

Default behavior:

- Include high and medium confidence.
- Exclude low confidence unless explicitly enabled.

## Evidence

Evidence should include:

- relative file path
- line
- optional column
- optional snippet

Line numbers are 1-based.

## Acceptance

- Unit tests cover surface key formatting.
- Unit tests cover confidence filtering.
- Extractor fixtures can assert facts without caring about SQLite.
- The model can represent both plugin usage and Shopware definitions.

## Out Of Scope

- Specific language parsing.
- SQLite schema.
- Terminal formatting.
