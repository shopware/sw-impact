# Chunk 04: PHP Extractor

## Goal

Implement real PHP extraction for high-value Shopware surface usages and definitions using `tree-sitter-php`.

## Scope

- Parse PHP files.
- Build per-file namespace and `use` import map.
- Resolve Shopware FQCN references where possible.
- Emit usage facts for plugin indexing.
- Emit definition facts for Shopware changed files.
- Include file/line evidence.

## Extract High-Confidence Usages

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

## Extract Medium-Confidence Usages

- Method calls where receiver type can be locally inferred.
- Service IDs in strings.
- DAL entity names in strings.
- Repository service IDs.
- Route names in attributes or strings.

## Extract Definitions For Shopware Check

- Classes.
- Interfaces.
- Traits.
- Enums.
- Methods.
- Properties.
- Constants.
- Enum cases.
- Constructor signatures.
- Method signatures.
- Visibility.

## Implementation Notes

- Do not attempt whole-program type inference.
- Keep inference local to one file.
- Build import resolution before method-call work.
- Treat dynamic names as low confidence or unsupported in MVP.
- Keep parser code behind a simple extractor trait so fixtures can run directly.

## Fixtures

Add fixtures for:

- Namespaced class with imports.
- Aliased import.
- Extends/implements/trait use.
- Attribute referencing Shopware class.
- Constructor type hint.
- Method return type.
- Static call.
- Constant reference.
- Event subscriber map.
- Service ID string.
- Route attribute/string.
- Shopware class definition with method signature.

## Acceptance

- PHP fixture tests emit expected surface keys.
- Resolved FQCN references are high confidence.
- String-based IDs are medium confidence unless exact structured context makes them high confidence.
- Definition extraction can detect removed methods/classes when comparing two fixture versions.

## Out Of Scope

- Cross-file type inference.
- PHPStan-level analysis.
- Runtime container resolution.
