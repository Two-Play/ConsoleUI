# My Plugin

A plugin for ConsoleUI: library entries and a panel in the quick menu, over the session bus. The
contract is `docs/plugins.md` in the ConsoleUI repository. This template is CC0 (see `COPYING`):
your plugin may use any license.

```bash
cargo run      # serves the plugin on your session bus (the shell starts it by itself later)
busctl --user call @APP_ID@ /org/consoleui/Plugin1 org.consoleui.Plugin1 Panel
```

To see it in the shell, install the program and the desktop entry, then turn it on under
Settings → Plugins:

```bash
cargo build --release
install -Dm755 target/release/my-plugin ~/.local/bin/my-plugin
install -Dm644 my-plugin.desktop ~/.local/share/applications/@APP_ID@.desktop
```

Or as a Flatpak with the manifest `@APP_ID@.yml` (change the ID to your own first):

```bash
flatpak install --user flathub org.freedesktop.Sdk.Extension.rust-stable//25.08   # once
flatpak-builder --user --install --force-clean build-dir @APP_ID@.yml
```
