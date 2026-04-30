# Chunk 08: Impact Report

## Goal

Turn changed surfaces and index lookup results into a useful human-readable terminal report.

## Scope

- Group impacts by confidence and surface kind.
- Print changed surface key.
- Print change type.
- Print classification when known.
- Print affected plugin count.
- Print usage count.
- Print file/line evidence rows.
- Make no-impact output clear.
- Keep default exit code successful.

## Output Shape

```text
Shopware Impact Report
Base: upstream/trunk
Compared: working tree

High confidence impact

php:method:Shopware\Core\Checkout\Cart\CartService::recalculate
change: signature changed
classification: public
affected plugins: 18
usages: 41

  PluginA  src/Subscriber/CartSubscriber.php:42  method-call  high
  PluginB  src/Service/Foo.php:88                static-call  high
  PluginC  src/Checkout/Bar.php:21               type-hint    high
```

## Report Rules

- Make clear this is touch evidence, not proof of breakage.
- Show high and medium confidence by default.
- Hide low confidence unless requested and available in the index.
- Limit evidence rows per surface by default, with a flag to expand.
- Sort highest-impact surfaces first:
  1. confidence
  2. affected plugin count
  3. usage count
  4. surface key

## Suggested Options

```bash
--max-evidence-per-surface 20
--include-low-confidence
--only-high-confidence
```

## Implementation Notes

- Keep report formatting separate from lookup logic.
- Use stable sorting for deterministic tests.
- Add a compact summary at the top:
  - changed surfaces
  - impacted surfaces
  - affected plugins
  - evidence rows shown

## Acceptance

- Snapshot tests cover no impact, single impact, multiple confidences, and truncated evidence.
- Report includes enough information to inspect plugin files manually.
- Default exit code is zero even when impact is found.
- Output wording avoids claiming definite breakage.

## Out Of Scope

- JSON output unless added later.
- CI threshold failures.
- Rich TUI rendering.
