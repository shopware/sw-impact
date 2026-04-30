# Chunk 10: Signature Classification And Hardening

## Goal

Add signature-change classification and harden the full V1 workflow against corpus scale, malformed files, and common edge cases.

## Scope

- Compare PHP method signatures.
- Compare constructor signatures.
- Detect visibility changes.
- Detect added required parameters.
- Detect removed parameters.
- Detect return type changes.
- Detect property type changes where practical.
- Improve rename heuristics.
- Run full workflow performance pass.
- Document limitations.

## Change Kinds

Classify:

- `removed`
- `possible-rename`
- `signature-changed`
- `visibility-changed`
- `required-parameter-added`
- `parameter-removed`
- `return-type-changed`
- `definition-added`

Only removed and changed existing surfaces should drive impact reporting by default. Added surfaces can appear in verbose/debug output.

## Implementation Notes

- Represent signatures structurally before comparing.
- Keep comparison rules conservative.
- Avoid claiming source compatibility breaks when uncertain.
- For rename heuristics, use same kind plus similar name/signature/path proximity.
- Record unknown changes as generic changed surfaces only when evidence is strong.

## Hardening Checklist

- Malformed PHP file does not abort full index by default.
- Malformed Twig/config file does not abort full index by default.
- Large skipped files are counted.
- Unsupported file types are ignored quietly.
- Empty plugin directories do not fail the whole run.
- Duplicate plugin names are handled.
- Duplicate evidence rows are deduplicated or intentionally preserved.
- Interrupted writes do not leave a file that looks like a valid complete index.

## Performance Checks

Measure:

- Full index time on the current corpus.
- Index size with snippets.
- Index size without snippets.
- Query time for common surfaces.
- Check time for a representative Shopware PR.

## Acceptance

- Signature fixture tests cover required-parameter addition, parameter removal, return type change, and visibility change.
- Full workflow works on a meaningful corpus sample.
- Performance measurements are documented.
- Known limitations are documented.
- The MVP definition in `README.md` is satisfied.

## Out Of Scope

- Perfect backward-compatibility analysis.
- Full semantic PHP analysis.
- CI failure policy.
