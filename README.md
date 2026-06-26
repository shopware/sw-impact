# sw-impact

Breaking change and public (extension) API static analyzer CLI tool, purpose-built for Shopware commerce core developers.
Intended to be run in the [Shopware](https://github.com/shopware/shopware) repository CI workflows to prevent introducing breaking changes by accident.

> [!NOTE]
> Very much WIP and in a rewrite right now, to build things properly and start with vision part 1.
>
> If you are looking for the previously vibe coded prototype, which reported impacted
> extension usages for a breaking change (vision part 2), switch to the [`vibe-prototype` branch](https://github.com/shopware/sw-impact/tree/vibe-prototype)

## How to install

1. Checkout this Repo
2. Have [Rust](https://rust-lang.org/) installed
3. Run `cargo build --release`
4. Afterwards you can use your executable under `./target/release/sw-impact`


## How to use

In general run `sw-impact --help` for latest CLI use instructions.

1. Have the shopware codebase cloned and checkout a "base" branch you want to compare against
2. Run `sw-impact index ../path/to/shopware`
3. Make changes in your shopware codebase or checkout a different state
4. Run `sw-impact check ../path/to/shopware` and see the breaking change report

> [!NOTE]
> Step 1 and 2 might be simplified at some point, e.g. the tool could check it out itself

## Vision

1. Detect and flag any (possible) breaking change to the public (extension) API that the Shopware commerce core exposes
2. (Optional) show (possible) impact evidence based on real extensions using the API surface,
   utilizing the source code of all extensions published in our own extension store.

## Constraints

Some design decisions to keep things simple:

- All analysis only works on a per file basis to keep it simple and performant
- All parsing happens through tree-sitter, no custom / specialized parsers per language,
  which aren't based on tree-sitter
- `.js` files are still parsed by the `TypeScript` grammar, to keep things simple and
  reuse the tree-sitter queries
- Overall try to keep the codebase minimal and performant, it doesn't have to cover
  every edge case, especially if it doesn't exists in Shopware's commerce core right now
  - for example only support Vue options API for now, because composition API isn't really used in the core (yet)

## Features

Checkmarked means implemented.

- [ ] Admin Vue.js components (not marked `@private` / `@experimental` / `@internal` in top level comment)
  - [x] Component existence under their approximated registered name, either by:
    - `import template from './sw-model-editor.html.twig';`, using the last `.html.twig` import that is in the same directory
    - fallback to extract from the file path parent folder (mostly followed convention in commerce core)
  - [x] Emit event names existence
  - [x] Computed
    - [x] existence
    - [x] signature breaks (return type)
    - [ ] extended syntax (object with getter / setter)
  - [x] Methods
    - [x] existence
    - [x] signature breaks (arguments, return type, is async)
  - [x] Properties
    - [x] existence
    - [x] signature breaks (type, required)
    - [x] added required prop to existing (public) component
- [ ] Admin Twig templates
  - [ ] block existence
- [ ] Storefront Twig templates
  - [ ] block existence
- [x] Lint output formats
  - [x] Pretty human readable
  - [x] JSON (AI / downstream readable)
  - [x] GitHub workflow PR annotations

## Ideas

That might or might not be implemented at some point:

- [ ] `sw-impact search` command, to query the shopware codebase by tree-sitter query and file type
- [ ] `sw-impact inspect` command, to inspect the tree-sitter syntax tree of a given file
- [ ] further breaking change check ideas, also looking at our [Backward Compatibility](https://developer.shopware.com/docs/resources/guidelines/code/backward-compatibility.html#backward-compatibility)
      guidelines.
  - [ ] Entity definitions
  - [ ] (optional) HTTP API schemas (currently already covered by [Explore OpenAPI GH App](https://github.com/apps/explore-openapi))
  - [ ] (optional) PHP code (currently already covered by [Roave/BackwardCompatibilityCheck](https://github.com/Roave/BackwardCompatibilityCheck))

## Development tips

Basics (if you are new to Rust):
- Tests can be executed with `cargo test`
- Linter can be executed with `cargo clippy`
- Formatter can be executed with `cargo fmt`
- For iterating on changes, you can also execute e.g. `cargo run --release -- check ../shopware`

Advanced:
- Be familiar with [tree-sitter](https://tree-sitter.github.io/tree-sitter/index.html)
- For building and testing tree-sitter queries:
  - You can experiment with their [playground](https://tree-sitter.github.io/tree-sitter/7-playground.html)
  - Or in Neovim, run `:InspectTree`, you can open the a query editor by pressing `o`

## License

[MIT License](LICENSE)
