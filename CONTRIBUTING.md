# Contributing to ConsoleUI

Thanks for helping to build a console experience for Linux! This guide gets you from zero to a pull request.

## Ways to contribute

- **Report bugs**, especially controller or hardware quirks (include your pad model and GPU).
- **Share ideas** for the store, gamepad-friendly apps and the home theater in [Discussions](https://github.com/Two-Play/ConsoleUI/discussions).
- **Translate**: the source language is English, the catalogs are gettext files in `lang/<code>/LC_MESSAGES/consoleui.po`.
- **Write code**: pick a [good first issue](https://github.com/Two-Play/ConsoleUI/labels/good%20first%20issue) or an open story in [`BACKLOG.md`](BACKLOG.md).
- **Design**: icons, sounds, themes and layout ideas for the couch (10-foot UI) are very welcome.

Please open an issue before starting a big change so we can agree on the approach.

## Setup

```bash
git clone https://github.com/Two-Play/ConsoleUI.git && cd ConsoleUI
cargo run -- --windowed --dummy        # shell in a window with made-up entries
cargo test -p consoleui-core -p consoleui
```

The compositor crate needs extra `-devel` packages, so use the dev container for the whole workspace:

```bash
dev/toolbox.sh create
dev/toolbox.sh run cargo test --workspace
dev/toolbox.sh run cargo clippy --workspace --all-targets -- -D warnings
```

A tour of the code is in [`CLAUDE.md`](CLAUDE.md). It is written for AI assistants, but works fine as an architecture guide.

## Before you open a pull request (Definition of Done)

- `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings` pass
- Tests pass; new logic that does not need Slint goes into `crates/core` with unit tests
- The feature works with **gamepad and keyboard**
- UI changes: add a `--snapshot` screenshot to the PR (see the commands in `CLAUDE.md`)
- New UI strings are English, wrapped in `@tr(...)`/`tr(...)`, and added to the catalog
  (run `cargo test -p consoleui-core i18n`)
- Everything in the repository is written in **English**: code, comments, UI strings, docs
- Keep the shell independent of the compositor; they only talk through the control socket

## Safety notes

- **Never run `consoleui-compositor --drm` on your host display.** Use the test VM (`docs/dev-vm.md`).
- Run GPU tests (nested gamescope, compositors) one at a time.
- The Wi-Fi/Bluetooth pages change real system state. Use `--fake-network` for scripted tests.
- Use a scratch `XDG_DATA_HOME` when scripting actions so your own profiles stay untouched.

## Commits and pull requests

- Small, focused commits with a clear message (imperative mood, e.g. "Add search ranking for umlauts")
- Reference the backlog story if there is one (e.g. "Story 5.7")
- One topic per pull request; describe what changed and how you tested it

## License

By contributing you agree that your work is licensed under [GPL-3.0-or-later](LICENSE).
Code taken from other projects needs a compatible license and attribution in a comment
(Smithay's anvil/smallvil are MIT, niri is GPL-3.0-or-later; do not copy from GPL-3.0-only code).
