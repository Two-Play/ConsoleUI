# My App

An app for ConsoleUI: a window on any Linux desktop, fullscreen in the console session.

```bash
cargo run                                # keyboard: arrows, Enter, Esc; mouse and controller work too
cargo run -- --snapshot /tmp/my-app.png  # saves the window as PNG and quits
cargo test                               # src/page.rs holds the logic
```

To see it in the shell, install `my-app.desktop` and the binary; the shell lists it with the other
apps. Design rules and what the SDK offers: `docs/sdk.md` in the ConsoleUI repository.
