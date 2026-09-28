# Contributing to Waffle

Thanks for helping! Waffle aims to be tiny, fast and faithful to your files. Contributions that keep it that way are very welcome.

## Before you start

- **Bugs:** open an issue with a sample file if you can. Anonymise it, or make the smallest file that reproduces the problem. Say what you did, what you expected, and what happened.
- **Features:** open an issue first for anything larger than a small fix, so we can agree on the approach.
- **File fidelity bugs** (Waffle changed something it shouldn't have on save) are the highest priority. Please attach the original file and the saved one.

## Setup

Follow [docs/development.md](docs/development.md). In short:

```sh
make test   # must pass
make lint   # must be clean (rustfmt + clippy -D warnings)
make app    # builds build/Waffle.app
```

## Guidelines

- **Keep it light.**
  - No new dependencies without a good reason; `waffle-calc`, `waffle-numfmt` and `waffle-refs` have none on purpose.
  - Per-cell memory is the budget that matters: think in bytes per cell.
- **Keep saves faithful.** Anything that writes files needs a round-trip test in `crates/waffle-io/tests/roundtrip.rs`. Unedited parts must stay byte-identical.
- **One undo step per user action.** New editing operations go in `waffle-core::ops` using `begin`/`commit`.
- **Native first.** The UI should behave like a well-made Mac app: standard shortcuts, menus, and system colours for chrome.
- **Tests:**
  - Rust logic gets unit tests next to the code.
  - Behaviour across formats goes in `crates/waffle-io/tests`.
  - Swift bridge behaviour goes in `macos/Tests/BridgeTests`.
  - Larger UI flows can be added to the debug self-test.
- **Test files:** add small fixtures by extending `tools/corpus/generate.py`, keeping it deterministic, and document the facts they exercise in `testdata/README.md`.

## Pull requests

- Keep them focused. Separate refactors from behaviour changes.
- Describe what changed and how you tested it. Include before/after screenshots for UI changes.
- CI runs `make lint` and `make test` on macOS.

By contributing you agree that your contributions are licensed under the project's license.
