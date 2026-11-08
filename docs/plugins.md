# Plugins for ConsoleUI

A plugin adds to the shell itself, where an app (see `docs/sdk.md`) is a program of its own. Plugins can

- **add games and apps to the library** (another launcher, emulators, ...), and
- **show a panel in the quick menu** (controls the shell draws: toggles, sliders, choices, buttons, info lines).

A plugin is a program of its own that talks to the shell over the session bus. That keeps the shell stable when a
plugin crashes, lets a plugin run in a Flatpak sandbox, and works from any language. The bus interface below is the
contract; it does not depend on the ConsoleUI crates, so plugins under any license, closed ones too, can implement
it. New members are only ever added; a breaking change would get a new interface name (`Plugin2`).

The user turns every plugin on in the settings ("Plugins") before the shell starts it, and sees what it provides.
A plugin that adds library entries can start programs on the user's machine: that is what the user agrees to.

## Writing one

```bash
dev/new-plugin.sh my-plugin [directory]    # in a ConsoleUI checkout
```

creates a plugin from `templates/plugin`: a small Rust program (only `zbus`, no ConsoleUI crate) that offers a
library entry and a panel with one item of each kind, its desktop entry, a Flatpak manifest and a README. The
template is CC0, so the plugin may use any license. Its bus name and Flatpak ID is `org.example.<Name>`: change it
to one of your own domain in the three places the README lists. Any other language works the same way; only the
interface below counts.

As a Flatpak a plugin needs no permissions for the contract: a sandbox may own its app ID on the session bus, the
exported desktop entry starts it with `flatpak run`, and the shell runs the commands of library entries itself.
Artwork from a sandbox is an icon name that starts with the app ID (Flatpak exports those), not a path.

## Being found

A plugin installs a desktop entry into an `applications` directory (`/usr/share/applications`,
`~/.local/share/applications`, or the Flatpak export of these):

```ini
[Desktop Entry]
Type=Application
Name=Lutris
Comment=Games from Lutris in the library
Icon=lutris
Exec=consoleui-plugin-lutris
# Not an app of its own: desktop menus and the shell's library leave it out
NoDisplay=true
# The bus name it owns while it runs; with Flatpak it must start with the app ID
X-ConsoleUI-Plugin=org.consoleui.plugins.Lutris
# What it offers: library, panel
X-ConsoleUI-Plugin-Provides=library;
```

Flatpak (1.18) drops unknown keys such as `X-ConsoleUI-Plugin` from the entry it exports and leaves
`X-Flatpak=<app ID>`; the shell then reads them from the entry inside the installed app, so a Flatpak plugin needs
nothing extra. The same holds for the `X-ConsoleUI*` keys of apps (`docs/sdk.md`).

The shell starts `Exec` when the plugin is turned on and its bus name has no owner yet, waits up to 5 seconds for
the name, and stops the process again when the plugin is turned off (or asks it to with `Quit`). A plugin may also
be D-Bus activatable or run already.

## The interface

Object `/org/consoleui/Plugin1`, interface `org.consoleui.Plugin1`, on the plugin's bus name.

| Member | Type | Meaning |
|---|---|---|
| `Version` | property `u` | The protocol version the plugin speaks, `1` |
| `Entries()` | method → `aa{sv}` | The library entries (only with `library`) |
| `EntriesChanged` | signal | The shell asks for `Entries()` again |
| `Panel()` | method → `(saa{sv})` | Title and items of the quick menu panel (only with `panel`) |
| `Activate(s id, v value)` | method | The user changed or pressed item `id`; `value` as listed below |
| `PanelChanged` | signal | The shell asks for `Panel()` again while the panel is open |
| `Quit()` | method, optional | The plugin was turned off and should exit |

Unknown keys are ignored, missing optional keys take their default, so both sides can grow.

### Library entries

| Key | Type | Meaning |
|---|---|---|
| `id` | `s` | Unique within the plugin, stable across runs (the shell keeps the history under it) |
| `title` | `s` | Shown name |
| `kind` | `s` | `game` (default) or `app`, for the library filter |
| `command` | `as` | Program and arguments the shell runs on the host to start it, like `["lutris", "lutris:rungameid/3"]` |
| `cover`, `hero`, `logo`, `icon` | `s` | Optional artwork: an absolute path the host can read, or an icon name. A sandboxed plugin cannot hand out paths inside its sandbox |
| `last_played` | `x` | Optional, Unix time in seconds; sorts the entry among the recent ones |

The shell starts entries like its own: in a systemd scope, so it knows the windows, can quit the app and the
nested gamescope options of the context menu work.

### Panel items

| Key | Type | Meaning |
|---|---|---|
| `id` | `s` | Unique within the panel |
| `kind` | `s` | `toggle`, `slider`, `choice`, `button` or `info` |
| `label` | `s` | Name of the row |
| `detail` | `s` | Optional line below it |
| `value` | `v` | `toggle`: `b`; `slider`: `d`; `choice`: `s` (one of `choices`); `info`: `s` |
| `min`, `max`, `step` | `d` | `slider` only; default 0, 100 and 1 |
| `unit` | `s` | `slider` only, shown after the number, e.g. `%` or ` W` |
| `choices` | `as` | `choice` only, in order |

`Activate` gets the new value: `b` for a toggle, `d` for a slider, `s` for a choice and `b` (true) for a button.
The shell shows the new value at once; the plugin answers with `PanelChanged` if something else changed too.
A button that takes a while should say so in an `info` item.

## Trying a plugin

`crates/plugins` holds the plugins that come with ConsoleUI, built like any other crate; `--fake` lets them change
nothing on the system. `busctl --user introspect <bus name> /org/consoleui/Plugin1` shows what a running plugin
offers.
