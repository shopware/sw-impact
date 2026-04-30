# Chunk 07: Shopware Change Detection

## Goal

Implement `sw-impact check` input discovery and surface diffing for a local Shopware worktree.

## Scope

- Resolve merge base with `--base`.
- List changed files since merge base.
- Include committed changes.
- Include uncommitted tracked changes.
- Include untracked files.
- Read base and current content for changed files.
- Run existing extractors in definition mode.
- Diff base/current surface facts.
- Emit changed surfaces for index lookup.

## Git Behavior

For:

```bash
sw-impact check \
  --shopware /path/to/shopware \
  --base upstream/trunk \
  --index .sw-impact/plugins.sqlite
```

The command should:

1. Verify `/path/to/shopware` is a Git worktree.
2. Resolve merge base between `HEAD` and `upstream/trunk`.
3. Collect files changed from merge base to `HEAD`.
4. Add uncommitted tracked changes.
5. Add untracked files.
6. Ignore deleted files only after extracting their base definitions.

## Surface Diff

Detect first:

- Removed class/interface/trait/enum.
- Removed method/property/constant/enum case.
- Removed service ID or alias.
- Removed event class or event name.
- Removed route name.
- Removed DAL entity or field.
- Removed Twig template or block.
- Removed Administration component/route/state API.
- Removed Storefront JS plugin.

Then:

- Possible rename as remove/add heuristic.
- Signature changes.

## Implementation Notes

- Keep Git commands behind a small adapter for testability.
- Prefer `git` CLI initially over linking libgit2.
- Use temporary fixture repos for integration tests.
- For untracked files, only current definitions exist.
- For deleted files, only base definitions exist.
- For modified files, compare base and current definitions.

## Acceptance

- Fixture repo tests cover committed, uncommitted, untracked, modified, and deleted files.
- Removed PHP method from base is emitted as a changed surface.
- Removed Twig block from base is emitted as a changed surface.
- Changed surfaces can be passed to the SQLite lookup path.
- Normal PR-sized checks are measured against the performance target.

## Out Of Scope

- CI threshold exit codes.
- Customer/install weighting.
- Perfect rename detection.
