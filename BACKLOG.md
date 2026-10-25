# ConsoleUI – Product Backlog

## Produktvision

Eine Konsolen-Oberfläche im Stil von PS5/Xbox für Linux, geschrieben in Rust. Man bedient sie
komplett mit dem Gamepad, sie hat flüssige Animationen und zeigt alle Apps und Spiele
(Steam + Desktop-Apps) übersichtlich an. Sie startet als eigene Session direkt aus dem
Login-Manager oder TTY, so wie der Steam Deck „Game Mode“.

## Architektur-Entscheidung

| Option | Aufwand | Kontrolle über Animationen | Bewertung |
|---|---|---|---|
| **A) gamescope als Compositor + eigene Shell als Vollbild-Client** | gering–mittel | in der Shell voll, beim App-Wechsel begrenzt | **Start hiermit** |
| B) Eigener Wayland-Compositor mit [Smithay](https://github.com/Smithay/smithay) | sehr hoch | voll (auch Übergänge zwischen Apps, Overlays) | später, falls A an Grenzen stößt |

**Entscheidung:** Wir bauen in Phase 1 die Shell als eigenständige Rust-App, die auch auf dem
normalen Desktop im Fenster läuft, damit die Entwicklung schnell geht. In Phase 2 läuft sie in
gamescope (so macht es auch Steam im Game Mode). Phase 3 ist optional ein eigener
Smithay-Compositor als Hybrid: Shell, Overlays, Eingabe und Fokus im eigenen Compositor,
gamescope nur noch verschachtelt pro Spiel für Upscaling/HDR (siehe Epic 7). Weil die Shell vom Compositor getrennt ist, bleibt der Code der Shell
dabei erhalten.

**Tech-Stack (Vorschlag)**
- UI: **Slint** (deklarative Animationen, Wayland, später auch `linuxkms`-Backend ohne Compositor) – bereits aus TempRechner bekannt
- Gamepad: `gilrs`
- App-Erkennung: `freedesktop-desktop-entry` (.desktop-Dateien), `keyvalues-parser` (Steam VDF/ACF)
- System (Power, Lautstärke, Netzwerk): `zbus` (logind, NetworkManager), PipeWire via `wpctl`
- Async/Prozesse: `tokio`
- Optional Phase 3: `smithay`

## Definition of Done

- `cargo build` und `cargo clippy -- -D warnings` laufen ohne Fehler
- Das Feature ist komplett mit Gamepad **und** Tastatur bedienbar
- Animationen laufen auf der Zielhardware flüssig (60 fps)
- Die Akzeptanzkriterien sind manuell geprüft, Logik (Parser usw.) hat Unit-Tests

Schätzung in Story Points (SP): 1, 2, 3, 5, 8, 13. Ein Sprint dauert 2 Wochen.

---

## Epic 0 – Setup & Spikes

| ID | Story / Task | SP |
|---|---|---|
| 0.1 | Rust-Toolchain per `rustup` installieren (ist im Moment nicht im PATH), Workspace `ConsoleUI` mit Crates `shell` (Binary) und `core` (Logik/Lib) anlegen | 1 |
| 0.2 | Slint-Grundgerüst: Vollbildfenster, das sich mit `--windowed` als Fenster öffnen lässt | 1 |
| 0.3 | **Spike gamescope:** Wie steuert man Fokus/App-Wechsel? (X11-Atoms wie `GAMESCOPECTRL_BASELAYER_APPID`, `STEAM_GAME`, `GAMESCOPE_EXTERNAL_OVERLAY`; Verhalten bei nativen Wayland-Clients mit `--expose-wayland`). Ergebnis: kurzes Dokument und Proof of Concept | 5 |
| 0.4 | **Spike Slint-Performance:** 50 Kacheln mit Bildern, Scale-/Blur-Animationen, fps messen | 3 |

## Epic 1 – Home-Screen (Herzstück)

| ID | User Story | Akzeptanzkriterien | SP |
|---|---|---|---|
| 1.1 | Als Spieler sehe ich eine horizontale Reihe mit meinen zuletzt genutzten Apps | Die Kacheln liegen nebeneinander, die gewählte Kachel ist größer und hat einen Glow | 5 |
| 1.2 | Als Spieler navigiere ich mit Steuerkreuz/Pfeiltasten durch die Kacheln | Die Fokus-Animation dauert ca. 150–250 ms mit Easing, die Liste scrollt mit, sodass der Fokus links bleibt (PS5-Stil) | 3 |
| 1.3 | Als Spieler ändert sich der Hintergrund passend zur gewählten App | Das Hero-Artwork blendet als Crossfade über, eine Vignette liegt darüber, Titel und Infos stehen unten links | 5 |
| 1.4 | Als Spieler sehe ich eine Top-Bar mit Uhrzeit, Akku, WLAN und Controller-Status | Die Werte aktualisieren sich live | 3 |
| 1.5 | Als Spieler starte ich mit A/Enter die gewählte App | Es gibt eine Start-Animation (Kachel zoomt auf Vollbild), dann startet der Prozess | 3 |
| 1.6 | Als Spieler höre ich dezente UI-Sounds beim Navigieren und Starten | Die Sounds lassen sich abschalten | 2 |

## Epic 2 – App-Übersicht / Bibliothek

| ID | User Story | Akzeptanzkriterien | SP |
|---|---|---|---|
| 2.1 | Als Spieler werden meine installierten Desktop-Apps erkannt | `.desktop`-Dateien aus XDG-Pfaden inkl. Flatpak werden geparst, `NoDisplay` wird gefiltert | 3 |
| 2.2 | Als Spieler werden meine Steam-Spiele erkannt | `libraryfolders.vdf` und `appmanifest_*.acf` werden geparst, Start über `steam steam://rungameid/<id>` | 5 |
| 2.3 | Als Spieler sehe ich schöne Cover-Bilder | Steam-`librarycache` (Cover/Hero/Logo) wird genutzt, sonst das Icon des Themes, sonst ein Platzhalter mit Farbverlauf | 5 |
| 2.4 | Als Spieler öffne ich eine Bibliothek als Grid-Ansicht | Grid mit Gamepad-Navigation, Filter (Alle/Spiele/Apps), Sortierung (A–Z, zuletzt gespielt) | 5 |
| 2.5 | Als Spieler suche ich nach Apps | Bildschirmtastatur für das Gamepad, Live-Filter | 5 |
| 2.6 | Als System merke ich mir die zuletzt genutzten Apps | Persistenz als JSON/TOML in `$XDG_DATA_HOME/consoleui/` | 2 |
| 2.7 | Als Spieler kann ich Apps anpinnen oder ausblenden | Kontextmenü mit der Options-Taste | 3 |

## Epic 3 – Eingabe

| ID | User Story | SP |
|---|---|---|
| 3.1 | Gamepad-Eingaben (`gilrs`) werden in UI-Aktionen übersetzt (Up/Down/Left/Right/Confirm/Back/Menu/Guide), Wiederholung beim Gedrückthalten | 5 |
| 3.2 | Tastatur-Mapping auf dieselben Aktionen, damit man ohne Controller entwickeln kann | 1 |
| 3.3 | Glyphen der Buttons passen sich an den Controller-Typ an (Xbox/PlayStation) | 3 |
| 3.4 | Die Guide-/PS-Taste öffnet von überall die Shell bzw. das Quick-Menü | 3 (hängt von 0.3 ab) |

## Epic 4 – App-Lebenszyklus & gamescope-Integration

| ID | User Story | SP |
|---|---|---|
| 4.1 | Session-Skript: gamescope im Embedded-Modus (DRM) mit der Shell als Haupt-Client starten | 3 |
| 4.2 | Session-Datei unter `/usr/share/wayland-sessions/consoleui.desktop`, damit man die Session in SDDM/GDM auswählen kann | 2 |
| 4.3 | Gestartete Apps bekommen den Fokus, beim Beenden kehrt die Shell mit Animation zurück | 5 |
| 4.4 | Laufende Apps werden in der Shell angezeigt („Läuft gerade“), man kann zurückwechseln oder sie beenden | 5 |
| 4.5 | Prozesse laufen als eigener systemd-Scope (`systemd-run --user --scope`), damit Absturz und Beenden sauber gehen | 3 |
| 4.6 | Quick-Menü als Overlay über laufenden Spielen (Lautstärke, Helligkeit, zurück zur Shell, Spiel beenden) | 8 (Spike 0.3) |

## Epic 5 – System & settings

| ID | User Story | SP |
|---|---|---|
| 5.1 | Power menu: power off, restart, sleep (logind via `zbus`) | 3 |
| 5.2 | Set the volume (PipeWire) | 3 |
| 5.3 | Wi-Fi list and connecting (NetworkManager via D-Bus) | 8 |
| 5.4 | Pair Bluetooth controllers (BlueZ via D-Bus) | 8 |
| 5.5 | Settings page: theme/accent color, sounds, resolution | 5 |
| 5.6 | Notifications as toasts (download finished, controller connected) | 3 |
| 5.7 | **Done.** As a player in a household I pick my profile at the start ("Who's playing?", like the users of a PS5): several profiles within one Linux user, each with its own history, pinned/hidden entries, game options and settings in `$XDG_DATA_HOME/consoleui/profiles/<id>/`. Profiles can be added, renamed, recolored and deleted with the gamepad (name entry with the on-screen keyboard); "Switch profile" in the control center; the first profile takes over the existing files. Not a login manager: no passwords, no separate Linux users | 5 |

## Epic 6 – Polish & Animations

| ID | User Story | SP |
|---|---|---|
| 6.1 | Animated boot/start screen with logo | 2 |
| 6.2 | Transitions between pages (Home ↔ Library ↔ Settings) with slide/fade | 3 |
| 6.3 | Parallax or subtle motion in the background artwork | 3 |
| 6.4 | Theme system with central design tokens (colors, radii, timings) | 3 |
| 6.5 | Performance pass: load images asynchronously, thumbnail cache, frame-time overlay for debugging | 5 |
| 6.6 | i18n: UI strings in English as the source language, German translation (e.g. Slint's `@tr()` with gettext catalogs), language follows `LANG` and can be changed on the settings page; remaining German strings in the code are moved to English | 5 |
| 6.7 | **Done (Sprint 11).** Optional background music on the home screen like on the PS5: a generated ambient loop or the user's own files, fades out while an app is in front | 3 |
| 6.8 | **Done (Sprint 11).** Animation polish: focus ring with a gap, light sweep on focus, ripple on press, gliding tab underline, crossfading info area, progress indicator on the launch screen | 3 |

## Epic 7 (optional, phase 3) – Own Smithay compositor (hybrid)

**Approach:** We do not rebuild everything gamescope can do. Full parity (FSR/NIS, HDR, frame
limiter, many per-game special cases) would be roughly 110–170 SP plus a lot of maintenance. Our own
Smithay compositor takes over the parts we currently have to work around in gamescope: shell,
overlays, input, focus and the transitions between shell and app. Games run directly in the
compositor (through XWayland). Games that need upscaling, HDR or an FPS limiter are started in a
**nested gamescope** (`gamescope -- <game>`, gamescope as a client), so we get those expensive
features from gamescope per game (Story 7.10, done in Sprint 8).

What goes away: the `STEAM_GAME` atoms and the process mapping in `src/gamescope.rs`, the
`EVIOCGRAB` in `src/grab.rs`, the notification window that is 1 px narrower than the root window,
and the focus that gamescope does not hand back after the overlay. The shell stays a normal client.
The gamescope mode (`CONSOLEUI_GAMESCOPE=steam`) remains as a fallback.

Spike results and the plan for 7.2: [docs/spike-smithay.md](docs/spike-smithay.md). Estimates below
are updated after the spike.

| ID | Story | SP |
|---|---|---|
| 7.1 | Spike: build Smithay's `anvil` example, run the shell (Slint, Wayland) and a nested `gamescope` inside it — **done (Sprint 8)** | 5 |
| 7.2 | **Done (Sprint 9).** Minimal compositor `crates/compositor`: DRM/KMS + libinput + libseat (`--drm`) and winit (`--winit`, development), the shell as fullscreen client, control socket (`CONSOLEUI_COMPOSITOR`), relative-pointer/pointer-constraints/`drm_syncobj`, own session file next to the gamescope session | 13 |
| 7.3 | **Done (Sprint 9).** XWayland basics: `X11Wm`, fullscreen games, override-redirect windows, window → launched app mapping (reuses `consoleui_core::ownership`, fallback for windows without `_NET_WM_PID`) | 13 |
| 7.4 | Robust XWayland for Steam and games: Steam client, launcher and splash windows, focus changes, special cases from tests with real games | 13 |
| 7.5 | **Done (Sprint 10).** Focus and input in the compositor: Guide button always goes to the shell, the game gets no input while the quick menu is open (replaces `grab.rs`), keyboard focus returns cleanly | 5 |
| 7.6 | **Done (Sprint 10).** Shell as overlay above running apps: quick menu and toasts as shell toplevels placed on top by the compositor (Slint has no layer-shell), replaces the `OverlayWindow`/`NotificationWindow` tricks | 5 |
| 7.7 | **Done (Sprint 11).** Animated transitions between shell and app in the compositor (tile zooms into the real game image, animated return) | 8 |
| 7.8 | Direct scanout for the game in front (zero-copy, low latency), falling back to composition while an overlay is open | 5 |
| 7.9 | VRR (adaptive sync) and tearing for games, simple frame pacing (Smithay has VRR, but no tearing-control/async page flip yet) | 13 |
| 7.10 | Nested gamescope per entry (resolution, FSR/NIS/integer, FPS limit, HDR, fullscreen) from the context menu — **done (Sprint 8)**, not for Steam games | 5 |
| 7.11 | Multiple monitors/hotplug (choose the output, switch to the remaining one on unplug) | 5 |
| 7.12 | Screenshot via button combination (compositor reads back the frame into a PNG, toast with the path) | 3 |

About 98 SP in total, 7.1–7.6 (≈ 54 SP) as the first goal: a session that runs as well without
gamescope as it does today. Deliberately **not** in the compositor: own upscaling, HDR and color
management, FPS limiter — the nested gamescope (7.10) provides those. License decided before 7.2:
GPL-3.0-or-later, so code from niri (GPL-3.0-or-later) and anvil/smallvil (MIT) can be reused.

---|---|---|
| 7.1 | Spike: Smithay-Beispiel `anvil` bauen und auf einem TTY starten; prüfen, ob die Shell (Slint, Wayland) und `gamescope` verschachtelt darin laufen | 5 |
| 7.2 | Minimaler Compositor: DRM/KMS + libinput + libseat, die Shell als Vollbild-Client, eigene Session-Datei neben der gamescope-Session | 13 |
| 7.3 | XWayland-Grundlagen: `X11Wm`, Vollbild-Spiele, Fenster ohne Rahmen (Override-Redirect), Zuordnung Fenster → gestartete App (wiederverwendet `consoleui_core::ownership`) | 13 |
| 7.4 | XWayland robust für Steam und Spiele: Steam-Client, Launcher- und Splash-Fenster, Fokuswechsel, Sonderfälle aus Tests mit echten Spielen | 13 |
| 7.5 | Fokus und Eingabe im Compositor: Guide-Taste immer zur Shell, Spiel bekommt während des Quick-Menüs keine Eingaben (ersetzt `grab.rs`), Tastaturfokus kommt sauber zurück | 8 |
| 7.6 | Shell als Overlay über laufenden Apps (Quick-Menü und Toasts als Layer-Shell-Flächen, ersetzt `OverlayWindow`/`NotificationWindow`-Tricks) | 8 |
| 7.7 | Animierte Übergänge zwischen Shell und App direkt im Compositor (Kachel zoomt in das echte Spielbild, Rückkehr mit Animation) | 8 |
| 7.8 | Direct Scanout für das Spiel im Vordergrund (Zero-Copy, niedrige Latenz) und Fallback auf Komposition, sobald ein Overlay offen ist | 8 |
| 7.9 | VRR (Adaptive Sync) und Tearing-Control für Spiele, einfaches Frame-Pacing | 8 |
| 7.10 | Verschachteltes gamescope pro Spiel: Option im Kontextmenü/Spiel-Einstellungen (Auflösung, FSR/NIS, FPS-Limit, HDR), wird in `Collection`/Einstellungen gespeichert und beim Start als `gamescope -W … -H … -F fsr -r … --` vorangestellt | 5 |
| 7.11 | Mehrere Monitore/Hotplug (Ausgabe wählen, beim Abstecken auf den verbleibenden wechseln) | 5 |
| 7.12 | Screenshot per Tastenkombination (Compositor rendert in eine Datei, Toast mit Pfad) | 3 |

Zusammen ≈ 97 SP, davon 7.1–7.6 (≈ 60 SP) als erstes Ziel: eine Session, die ohne gamescope
genauso gut läuft wie heute. Bewusst **nicht** im Compositor: eigenes Upscaling, HDR und
Farbmanagement, FPS-Limiter. Das liefert das verschachtelte gamescope (7.10).

---

## Sprint plan (proposal)

| Sprint | Goal | Stories |
|---|---|---|
| **1** | "It moves": home screen with dummy data and smooth navigation | 0.1, 0.2, 0.4, 1.1, 1.2, 3.2 |
| **2** | "Real apps": detect desktop and Steam apps, show them with covers and launch them | 2.1, 2.2, 2.3, 1.3, 1.5, 2.6 |
| **3** | "Couch-ready": gamepad control and console session with gamescope | 3.1, 0.3, 4.1, 4.2, 4.3, 1.4 |
| **4** | "Library": grid, search, running apps | 2.4, 2.5, 4.4, 4.5, 3.3 |
| **5** | "System": power, volume, quick menu | 5.1, 5.2, 3.4, 4.6 |
| **6** | "Polish": animations, settings, performance | 6.x, 5.5, 5.6, 1.6 |
| **7** | Wi-Fi/Bluetooth, context menu, notifications over games, performance | 5.3, 5.4, 2.7, 5.6, 6.3, 6.5 |
| **8** | "Hybrid start": Smithay spike, nested gamescope per entry, i18n | 7.1, 7.10, 6.6 |
| **9** | "Own compositor": minimal compositor, XWayland, test VM, license | 7.2, 7.3 |
| **10** | "Session without gamescope": input isolation, overlays, keyboard layout, DRM checks | 7.5, 7.6 |
| **11** | "Transitions": shell ↔ app animated in the compositor, animation polish, background music | 7.7, 6.7, 6.8 |
| 12+ | Steam and launchers (with the user), then latency | 7.4 → 7.8, 7.9, 7.11, 7.12 |

## Sprint-Ergebnisse

### Sprint 1 – „Es bewegt sich“ (erledigt: 0.1, 0.2, 0.4, 1.1, 1.2, 3.2)

- Workspace mit `consoleui-core` (Logik, 8 Tests) und `consoleui` (Slint-Shell, 3 Tests)
- Home-Screen: Kachelreihe mit Fokus-Animation (220 ms, `ease-out-quart`), Glow, „Drück“-Effekt,
  Hintergrund in der Akzentfarbe, Top-Bar mit Uhr, Infobereich, Steuerungshinweise
- Tastatur: Pfeile/WASD, Enter/Leertaste, Esc/Backspace, Tab/M, Strg+Q zum Beenden
- Noch nicht drin: Starten von Apps (Sprint 2). Hoch/Runter tun noch nichts.

**Spike 0.4 – Ergebnis** (`--bench`, Release, KDE Wayland, 5120×1440, femtovg-Renderer)

| Kacheln | Ø fps | Renderzeit Ø | max |
|---|---|---|---|
| 12 | 114 | 5,3 ms | 18 ms |
| 50 | 112 | 5,8 ms | 32 ms |
| 100 | 109 | 6,0 ms | 28 ms |

Die Renderzeit hängt kaum von der Anzahl der Kacheln ab, bezahlt wird vor allem die Fläche
(Vollbild-Verläufe und Schatten). Die Max-Werte kommen vom ersten Frame (Texturen hochladen).
Ein echter Weichzeichner (Blur) für Hintergründe ist in Slint nicht eingebaut. Für 1.3 nehmen wir
deshalb ein vorab verkleinertes oder weichgezeichnetes Bild. Auf schwacher Hardware (Steam
Deck o. Ä.) sollten wir später den Skia-Renderer vergleichen (Story 6.5).

### Sprint 2 – „Echte Apps“ (umgesetzt: 2.1, 2.2, 2.3, 1.3, 1.5, 2.6)

- **Steam (2.2):** eigener VDF-Parser, alle Bibliotheken aus `libraryfolders.vdf`, nur vollständig
  installierte Spiele. Proton, Runtimes, Redistributables und Soundtracks werden ausgefiltert,
  Duplikate über mehrere Bibliotheken zusammengeführt. Start über `steam steam://rungameid/<id>`.
- **Desktop-Apps (2.1):** eigener `.desktop`-Parser mit deutschen Namen. Aufgenommen werden die
  Kategorien Game/AudioVideo/Audio/Video/Network/Graphics, ausgeschlossen
  Settings/System/Development/Office. Steam-eigene Spielverknüpfungen werden übersprungen, Icons
  kommen aus dem KDE-Theme, sonst aus hicolor oder pixmaps.
- **Artwork (2.3):** Steam-Cover, Logo (ersetzt den Titeltext) und Hero aus `appcache/librarycache`,
  sowohl im neuen als auch im alten Format. Die Akzentfarbe wird aus dem Cover berechnet.
- **Hintergrund (1.3):** Das Hero-Bild wird erst beim Fokussieren geladen und über zwei Ebenen
  überblendet (500 ms).
- **Starten (1.5):** Die Kachel zoomt auf Vollbild („wird gestartet …“), der Prozess läuft in einer
  eigenen Prozessgruppe, und nach 2,6 s kehrt die Shell zurück. Die echte Fokus-Übergabe folgt mit 4.3.
- **Historie (2.6):** `~/.local/share/consoleui/history.json`. Gestartete Apps rutschen nach vorn,
  die Reihe ist nach „zuletzt genutzt“ sortiert, danach alphabetisch.
- Messwert mit echter Bibliothek (69 Kacheln, Release): Ø 105 fps, Renderzeit Ø 6,3 ms, max 17,7 ms.
- Bekannt: Einige AppImage-Icons ohne Dateiendung lassen sich nicht laden. Die Kachel zeigt dann
  das Monogramm. Mit 69 Einträgen wird die Reihe lang, was für die Bibliothek (2.4) spricht.

### Sprint 3 – „Couch-tauglich“ (umgesetzt: 3.1, 0.3, 4.1, 4.2, 4.3, 1.4, teilweise 3.3)

- **Spike gamescope (0.3):** siehe [docs/spike-gamescope.md](docs/spike-gamescope.md). Ohne `-e`
  bekommt das neueste Fenster den Fokus, nach dem Schließen ist die Shell automatisch zurück.
- **Gamepad (3.1):** gilrs, alle 8 ms abgefragt. Steuerkreuz und linker Stick (mit Hysterese)
  navigieren, mit Wiederholung beim Halten (350 ms, dann 120 → 70 ms). Kreuz/A bestätigt,
  Kreis/B geht zurück, Options/PS öffnen das Menü (vorerst ohne Funktion). Geräte ohne
  SDL-Mapping werden ignoriert, z. B. die Joystick-Schnittstelle der Keychron-Tastatur.
- **Top-Bar (1.4):** Controller mit Akkustand, Netzwerk (Ethernet/WLAN/offline aus sysfs) und
  Laptop-Akku (nur wenn vorhanden), dazu die Uhr. Alles wird jede Sekunde aktualisiert.
- **Tastensymbole (3.3, Teil):** Bei PlayStation-Controllern ✕/○, bei Xbox und anderen A/B, mit
  Tastatur ⏎/Esc. Es gibt noch keine eigenen Glyph-Grafiken.
- **Rückkehr (4.3):** Der Startbildschirm bleibt offen, solange die App läuft. Desktop-Apps werden
  über das Prozessende erkannt, Steam-Spiele über `RunningAppID` in `~/.steam/registry.vdf`, mit
  90 s Timeout, falls das Spiel nie startet. Danach fährt die Shell animiert zurück. Mit
  Kreis/Esc kommt man vorher zur Übersicht. Steam wird mit `-silent` gestartet.
- **Session (4.1/4.2):** `session/consoleui-session`, `session/consoleui.desktop` und
  `session/install.sh` (mit sudo ausführen). Verschachtelt in gamescope getestet. Der Start vom
  TTY bzw. Login-Bildschirm ist noch nicht getestet.
- Noch offen: das echte Bedienen mit dem DualSense und die Session auf echter Hardware
  (DRM-Backend) testen.

### Sprint 4 – „Bibliothek“ (umgesetzt: 2.4, 2.5, 4.5, 3.3, Teile von 4.4)

- **Fehler aus Sprint 3 behoben:** gilrs liest den Controller direkt vom Gerät. Ohne Fensterfokus
  ignoriert die Shell deshalb jetzt alle Gamepad-Eingaben (`FocusScope.has-focus`), damit Tasten
  im Spiel keine Aktionen in der Shell auslösen. In gamescope mit einem zweiten Fenster getestet.
- **Navigation:** Die Tabs „Home“ und „Bibliothek“ wechseln mit L1/R1 bzw. Q/E. Die Home-Reihe
  zeigt die 12 zuletzt genutzten Apps und am Ende die Kachel „Bibliothek“. Dreieck bzw. `/` öffnet
  von Home und Bibliothek aus die Suche. Seitenwechsel blenden über und gleiten (200 ms).
- **Bibliothek (2.4):** Raster mit Titeln, die Spaltenzahl richtet sich nach der Fensterbreite.
  Quadrat/F schaltet den Filter (Alle/Spiele/Apps) um, Options/Tab die Sortierung (zuletzt
  genutzt / A–Z, Umlaute richtig einsortiert; seit Sprint 7 im Kontextmenü).
- **Suche (2.5):** QWERTZ-Bildschirmtastatur mit Ziffern, Leerzeichen, ⌫ und „Fertig“. Mit der
  echten Tastatur kann man direkt tippen. Die Suche ignoriert Groß-/Kleinschreibung und Umlaute,
  jedes Suchwort muss vorkommen. Die Reihenfolge ist: Titelanfang, dann Wortanfang, dann irgendwo
  im Titel. Die Treffer stehen als Kachelreihe darunter, ohne Suchtext die zuletzt genutzten Apps.
- **Tastensymbole (3.3):** gezeichnete Symbole in den Controller-Farben (✕ ○ △ □ / A B X Y,
  L1/R1 bzw. LB/RB, ≡), mit Tastatur die Tastennamen. Die Hinweisleiste passt sich an Bildschirm
  und Auswahl an.
- **systemd-Scope (4.5):** Desktop-Apps laufen über `systemd-run --user --scope` in einer eigenen
  Unit (`consoleui-app-*.scope`). Ohne User-Bus wird direkt gestartet (Prozessgruppe).
- **Laufende App (4.4, Teil):** „● LÄUFT“-Badge und „Läuft gerade“ als Unterzeile. Options/Tab
  in Home beendet die App nach einer Rückfrage: Desktop-Apps über `systemctl --user stop`,
  Steam-Spiele per SIGTERM an Prozesse mit `SteamAppId=<id>`. Kreis/Esc schließt den
  Startbildschirm, die App wird weiter verfolgt.
  **Offen für Sprint 5:** zu einer laufenden App zurückwechseln (gamescope-Fokus) und mehrere
  laufende Apps gleichzeitig verfolgen.
- **Entwickler-Optionen:** `--start home|library|search`, `--query <text>`, `--snapshot <png>`
  (rendert das eigene Fenster, ohne den Bildschirm abzufotografieren).
- Messwert: Home mit Bibliothekskachel Ø 105 fps, Renderzeit Ø 5,7 ms.
- Nicht manuell getestet: Beenden-Dialog und Badge mit einer echten App sowie die
  Start-Animation aus Bibliothek und Suche.

### Sprint 5 – „System“ (umgesetzt: 4.4 Rest, 4.6, 3.4, 5.1, 5.2)

- **Spikes:** [docs/spike-gamescope.md](docs/spike-gamescope.md) (Spike 2 und 3). Zurückwechseln
  geht nur im Steam-Modus von gamescope, ein echtes Overlay mit einem ARGB-Fenster und
  `STEAM_OVERLAY` vor dem Mappen.
- **Session im Steam-Modus:** `session/consoleui-session` startet `gamescope -e` und setzt
  `CONSOLEUI_GAMESCOPE=steam`. Die Shell übernimmt die Rolle von Steam: Sie gibt jedem neuen
  Fenster per x11rb ein `STEAM_GAME`-Atom. Die Zuordnung läuft über `_NET_WM_PID`, dann
  `SteamAppId` in der Umgebung oder die systemd-Scope-Unit bzw. die Elternprozesse. Fenster ohne
  Zuordnung gehen an die zuletzt gestartete App. Mit `GAMESCOPECTRL_BASELAYER_APPID` bestimmt die
  Shell, wer vorne ist.
- **Mehrere laufende Apps (4.4):** Alle werden verfolgt und haben ein Badge. ✕ auf einer
  laufenden App holt sie zurück nach vorn („Fortsetzen“), ohne gamescope zeigt sie den
  Startbildschirm „läuft“. Ist das App-Fenster da, verschwindet der Startbildschirm.
- **Steuerzentrale (4.6, 3.4):** Die PS-Taste (Tastatur: F1) öffnet sie von überall, auch während
  ein Spiel vorne läuft. In gamescope ist sie ein eigenes transparentes Overlay-Fenster über dem
  Spiel, ohne gamescope eine Ebene im Hauptfenster. Inhalt: Fortsetzen, Home, Beenden (mit
  Spiel), Lautstärke, Standby, Neustart, Ausschalten, Abmelden. Folgenschwere Aktionen brauchen
  einen zweiten Druck („Wirklich?“).
- **Lautstärke (5.2):** `wpctl` auf `@DEFAULT_AUDIO_SINK@`, ▲/▼ in 5-%-Schritten, ✕ schaltet stumm.
- **Energie (5.1):** `systemctl suspend|reboot|poweroff` (logind/polkit, ohne root).
- **Entwickler-Optionen:** `--actions` (simulierte Eingaben) und `--dummy-launch <befehl>`, damit
  sich der Ablauf in gamescope automatisch testen lässt.
- Getestet: der komplette Ablauf verschachtelt in gamescope `-e` mit `glxgears` (Screenshots per
  `GAMESCOPECTRL_REQUEST_SCREENSHOT=2`), das Quick-Menü im Desktop-Fenster per `--snapshot`.
- **Nicht getestet:** echte Steam-Spiele und der Steam-Client in der Steam-Modus-Session, die Session
  auf echter Hardware (DRM), Lautstärke regeln und Energieaktionen auslösen.
- **Bekannte Grenze:** Während die Steuerzentrale offen ist, bekommt das Spiel die Controller-Tasten
  weiterhin mit, denn gilrs und das Spiel lesen dasselbe Gerät. Abhilfe wäre ein exklusiver Zugriff
  (`EVIOCGRAB`) oder ein virtueller Controller, ähnlich wie bei Steam Input.

### Sprint 6 – „Polish“ (umgesetzt: 6.1, 6.5 teilweise, 5.5, 5.6, 1.6, exklusiver Controller)

- **Einstellungen (5.5):** dritter Tab „Einstellungen“ mit Akzentfarbe (Weiß, Blau, Violett,
  Grün, Orange, Rot), Animationen an/aus (alle Dauern 0), Klängen an/aus und Tastensymbolen
  (automatisch/PlayStation/Xbox). ▲▼ wählt, ◀▶ ändert. Gespeichert wird sofort in
  `~/.config/consoleui/settings.json`, fehlende Felder bekommen Standardwerte. Die Akzentfarbe
  färbt Fokusrahmen, Buttons, Tasten und Karten.
- **Klänge (1.6):** werden zur Laufzeit erzeugt (Sinus mit Oberton und Hüllkurve, keine Dateien)
  und über einen dauerhaften `pw-cat --raw` abgespielt. Ein eigener Thread schreibt, die Queue ist
  klein, dicht folgende Fokusklänge fallen weg. Welcher Klang kommt, ergibt sich aus der Änderung:
  Fokus, Seite, Steuerzentrale, Start oder Bestätigen.
- **Boot-Animation (6.1) und Laden im Hintergrund (6.5):** Logo mit pulsierendem Lichtschein und
  Ladepunkten, mindestens 1,2 s. Währenddessen laufen Erkennung und Bilddekodierung (`image`-Crate,
  mehrere Threads, Cover auf höchstens 480×720 verkleinert, Akzentfarbe) im Hintergrund.
  Benchmark, Skript und Snapshot starten erst danach.
- **Hinweise (5.6):** Toast oben rechts bei „Controller verbunden/getrennt“, „Start
  fehlgeschlagen“ und schwachem Controller-Akku (≤ 15 %, einmal pro Entladung).
- **Exklusiver Controller:** Solange die Steuerzentrale offen ist, sperrt die Shell die
  evdev-Geräte des Controllers per `EVIOCGRAB` und liest sie selbst (Steuerkreuz, Stick, ✕, ○,
  Options, PS). Das Spiel bekommt dann nichts mit. Getestet: Ein zweiter Grab-Versuch scheitert
  mit EBUSY, solange das Menü offen ist, vorher und nachher ist das Gerät frei.
- Messwert: Ø 112 fps, Renderzeit Ø 5,8 ms, max 15 ms (Home, echte Bibliothek).
- **Nicht getestet:** die Bedienung mit echtem Controller während des Grabs, das Hören der
  Klänge, Toasts bei echtem Ein- und Ausstecken.
- **Grenzen:** Spiele, die den Controller über hidraw lesen (Steam Input, manche SDL-Spiele),
  sehen ihn trotz Grab. Toasts erscheinen nur, wenn die Shell sichtbar ist, nicht über einem
  laufenden Spiel (seit Sprint 7 behoben).

### Nachtrag – Fehler „Controller reagiert nach Home nicht mehr“

- Ursache: Nach dem Overlay gibt gamescope den X11-Tastaturfokus nicht an die Shell zurück, obwohl
  sie wieder vorne ist. Die Controller-Sperre aus Sprint 4 hing am Fensterfokus, deshalb ging nur
  noch die PS-Taste.
- Lösung: In gamescope entscheidet jetzt `Shell::shell_in_front` (keine App vorne, Startbildschirm
  offen oder die App vorne hat noch kein Fenster), ob Controller-Eingaben an die Shell gehen.
  Die Tastatur holt sich den Fokus per `XSetInputFocus` zurück (spätestens nach 500 ms).
- Dazu `session/consoleui-nested`: simuliert die Session in einem Desktop-Fenster.

### Nachtrag – Fehler „Steam-Spiel gilt nach dem Beenden noch minutenlang als laufend“

- Ursache: Das Spielende wurde an `RunningAppID` in `~/.steam/registry.vdf` erkannt, und das setzt
  Steam oft erst Minuten später zurück.
- Lösung: `steam::running_games()` sucht in `/proc` nach Prozessen mit `SteamAppId=<id>` (setzt Steam
  für jedes Spiel, auch unter Proton). Gibt es 2 s lang keinen, ist das Spiel beendet.
  `steam::stop_game` nutzt dieselbe Suche.

### Sprint 7 – Kontextmenü (umgesetzt: 2.7)

- **Kontextmenü (2.7):** Options/Tab öffnet auf dem fokussierten Eintrag in Home und Bibliothek
  eine kleine Karte mit „Anheften“/„Lösen“, „Ausblenden“ und bei laufenden Apps „Beenden“ (mit der
  bekannten Rückfrage). ▲▼ wählt, ✕ führt aus, ○ schließt. Der „•••“-Knopf in Home steht dafür,
  die Hinweisleiste zeigt „Optionen“.
- **Angeheftet:** steht in der Home-Reihe immer vorn (in Anheft-Reihenfolge, zählt nicht gegen die
  12 zuletzt genutzten) und trägt eine Nadel oben rechts in der Akzentfarbe. In der Bibliothek gilt
  die normale Sortierung.
- **Ausgeblendet:** verschwindet aus Home, Bibliothek und Suche. Der Bibliotheksfilter hat jetzt
  den Wert „Ausgeblendet“, dort bietet das Kontextmenü „Wieder anzeigen“.
- **Sortierung:** Weil Options jetzt das Kontextmenü öffnet, steht „Sortierung: …“ in der
  Bibliothek als letzter Menüpunkt (✕ schaltet um, das Menü bleibt offen). Eine freie Taste gab es
  dort nicht mehr.
- Gespeichert in `$XDG_DATA_HOME/consoleui/collection.json` (Eintrags-IDs, tolerant geladen, atomar
  geschrieben); mit `--dummy` wird wie bei der Historie nichts gespeichert.
- Geprüft per `--snapshot` (Menü in Home/Bibliothek, Nadel, Filter „Ausgeblendet“, Suche ohne
  ausgeblendeten Eintrag, „Beenden“ mit `--dummy-launch "sleep 15"`). **Nicht getestet:** mit
  echter Bibliothek und echtem Controller.

### Sprint 7 – WLAN & Bluetooth (umgesetzt: 5.3, 5.4)

- **Einstieg:** zwei neue Zeilen „WLAN“ und „Bluetooth“ in den Einstellungen. Sie zeigen den
  Zustand („Kein Adapter“, Netzname, „1 Gerät verbunden“, „Aus“) und öffnen mit ✕ eine Unterseite,
  ○ führt zurück. Die Unterseiten sind ein Zustand der Einstellungsseite (kein neuer `Screen`).
- **WLAN (5.3):** Ein/Aus-Zeile, darunter die Netze mit Signalbalken, Schloss für gesicherte Netze
  und „Verbunden“-Badge (pro SSID zusammengefasst, verbundenes Netz zuerst, dann nach Signal).
  ✕ verbindet bzw. trennt, □ sucht neu. Gesicherte Netze ohne gespeichertes Profil öffnen die
  Passwort-Eingabe: dieselbe Bildschirmtastatur wie die Suche, mit einer neuen Umschalttaste ⇧
  (Großbuchstaben und Sonderzeichen), ≡ zeigt das Passwort an, auch die echte Tastatur tippt.
  Ohne Adapter steht dort „Kein WLAN-Adapter“.
- **Bluetooth (5.4):** Ein/Aus-Zeile mit Adaptername, □ startet bzw. beendet die Gerätesuche.
  Geräte mit Art (Controller, Audio, Tastatur …, aus BlueZ-`Icon`, Class of Device oder Name),
  Controller sind mit Grafik und Rahmen hervorgehoben und stehen vorne. ✕ koppelt (Pair + Trusted
  + Connect), verbindet oder trennt; ≡ zweimal entfernt ein gekoppeltes Gerät. Für die Kopplung
  registriert die Shell einen eigenen BlueZ-Agenten („NoInputNoOutput“, nimmt alles an), damit es
  auch in der gamescope-Session ohne KDE-Agent klappt. Unbenannte, ungekoppelte Geräte werden
  ausgeblendet.
- **Technik:** NetworkManager und BlueZ über D-Bus mit `zbus` (blockierend) in einem eigenen
  Thread (`consoleui_core::network::spawn`), die UI holt die Antworten alle 100 ms per Timer ab.
  Neu gelesen wird nur, solange die Einstellungen offen sind (alle 3 s, bei Suche/Auftrag 1 s).
  Ergebnisse kommen als Toast („Verbunden mit …“, „Kopplung fehlgeschlagen“).
  Logik ohne D-Bus (Modelle, Sortierung, Flags, Seitenzustand, Simulation) liegt in `core` mit Tests.
- **Entwickler-Option:** `--fake-network` simuliert WLAN und Bluetooth (Netze, Passwortprüfung,
  Geräte bei der Suche, eine Kopplung, die immer scheitert) und ändert nichts am System.
  Snapshots mit `--actions` warten jetzt, bis das Skript durch ist.
- Getestet: alle Abläufe mit `--fake-network` per `--snapshot` (Liste, Passwort, Verbinden,
  Suche, Koppeln, Fehler, Entfernen). Auf diesem PC (nur Ethernet) nur lesend: Die WLAN-Seite
  zeigt „Kein WLAN-Adapter“, die Bluetooth-Seite den ausgeschalteten Adapter „fedora-pc“.
- **Nicht getestet (keine Hardware bzw. bewusst keine Systemänderung):** echtes Verbinden mit
  einem WLAN (`AddAndActivateConnection`, WPA3/SAE, Löschen des Profils bei falschem Passwort),
  WLAN ein/aus und `RequestScan`, Bluetooth ein/aus, Gerätesuche, Koppeln mit dem eigenen Agenten
  (DualSense, Xbox), Verbinden/Trennen/Entfernen. Der Agent ist bewusst einfach (kein PIN-Dialog,
  alte Geräte bekommen „0000“). WEP und WPA-Enterprise werden abgelehnt.

### Sprint 7 – Hinweise über Spielen (5.6 Rest)

- **Toasts über laufenden Apps:** Ist in gamescope eine App vorne (`Shell::shell_in_front` ist
  falsch), erscheint der Hinweis in einem eigenen, transparenten `NotificationWindow`
  (`ui/notification.slint`, nutzt `ui/toast.slint`). Es bekommt vor dem Mappen `STEAM_OVERLAY=1`,
  aber kein `STEAM_INPUT_FOCUS`, und ist ein Pixel schmaler als das Root-Fenster. Damit wählt
  gamescope es als Benachrichtigungsfenster: immer obenauf, nie fokussiert. Nach der Anzeigedauer
  blendet der Hinweis aus und das Fenster wird versteckt. Ist die Shell vorne oder läuft sie ohne
  gamescope, bleibt es beim Toast im Hauptfenster.
- Entwicklungshilfe: Der `--actions`-Schritt `toast` zeigt einen Testhinweis.
- Verschachtelt getestet (960×540, `glxgears`): Der Hinweis steht oben rechts über dem Spiel,
  `GAMESCOPE_FOCUSED_WINDOW` und der X11-Eingabefokus bleiben beim Spiel. Auch zusammen mit der
  offenen Steuerzentrale sind beide zu sehen, und der Fokus liegt beim Overlay. Steuerzentrale →
  Home geht danach, und ein Hinweis auf dem Home-Screen erscheint wieder im Hauptfenster.
- **Grenzen:** gamescope ignoriert die Position des Benachrichtigungsfensters. Es zentriert das
  Fenster und skaliert es auf die Ausgabe, deshalb ist es fast bildschirmgroß (feste Größe über
  min = max in den Size-Hints, sonst zieht gamescope es auf Root-Größe). Ältere gamescope-Versionen
  unterscheiden Overlay und Benachrichtigung nur nach der Breite (> 1200 px = Overlay). Dort wäre
  das Fenster ein Overlay ohne Eingabe, also zu sehen, aber es könnte das Quick-Menü verdecken.
  Nicht auf echter Hardware getestet.

### Nachtrag – Fehler „Spiel bekommt nach der Steuerzentrale keine Tastatureingaben“

- Gefunden beim Test der Hinweise: Nach dem Schließen des Overlays gibt gamescope den
  Tastaturfokus auch der App vorne nicht zurück (wie schon bei der Shell).
- Lösung: `Gamescope::focus_app` setzt ihn auf das Fenster der App, falls er nicht in einem
  ihrer Fenster oder Unterfenster liegt (läuft alle 500 ms mit `restore_keyboard_focus`).

### Sprint 7 – Performance & Parallax (umgesetzt: 6.5 Rest, 6.3, Icon-Fehler)

- **Heroes im Hintergrund (6.5):** `src/hero.rs` dekodiert die Steam-Heroes (bis 3840 px breit) auf
  zwei Worker-Threads und verkleinert sie auf höchstens 1920×1080. Das alte Bild bleibt stehen, bis
  das neue fertig ist, dann blenden die beiden Ebenen über. Die Nachbarn der fokussierten Kachel
  (±1, +2, in der Bibliothek auch die Zeile darüber/darunter) werden vorab geladen, veraltete
  Aufträge verfallen beim nächsten Fokuswechsel, der Cache hält die letzten 12. Abgeholt wird mit
  einem `Timer::single_shot`, der sich nur bei offenen Aufträgen neu plant.
- **Parallaxe (6.3):** `ui/hero.slint` (`HeroLayer`) ist 6 % größer als das Fenster und wandert mit
  der Fokusspalte um bis zu 3 % zur Gegenseite; beim Einblenden gleitet das Bild aus 5 % Zoom zurück
  (1,5 s). Beides animiert nur nach einer Änderung, ein ruhender Bildschirm zeichnet nicht neu. Mit
  „Animationen aus“ sind die Dauern 0 und die Verschiebung ist 0.
- **Icons ohne Dateiendung:** `consoleui_core::image_format::sniff` erkennt PNG, JPEG (Signatur) und
  SVG (Text) am Dateianfang; `tiles::load_icon` lädt damit z. B. `~/AppImages/.icons/designcraft`
  (alle sieben AppImage-Einträge zeigen jetzt ihr Icon statt des Buchstabens).
- **Benchmark:** `--bench` gibt zusätzlich den größten Frameabstand und die längste
  Eingabeverarbeitung im UI-Thread aus. Messung echte Bibliothek, Home, 4 Läufe abwechselnd
  (Rechner wenig belastet):

  | | vorher | nachher |
  |---|---|---|
  | Ø fps | 71–108 | 101–110 |
  | Renderzeit max | 14–52 ms | 14–20 ms |
  | max Frameabstand | 19–62 ms | 17–24 ms |
  | längste Eingabe im UI-Thread | 12–25 ms | 0,06–0,12 ms |

  Unter starker Last (andere Builds, Load ≈ 32) waren es vorher bis 730 ms im UI-Thread, nachher
  höchstens 7 ms.
- **Nicht getestet:** die Parallaxe in Bewegung (Snapshots zeigen nur den Endzustand), 4K-Ausgabe
  (dort ist ein 1920er-Hero etwas weicher).

### Sprint 8 – "Hybrid start" (done: 7.1, 7.10, 6.6)

Three agents worked in parallel in their own worktrees; the branches were merged into `main`.

- **Spike Smithay (7.1):** [docs/spike-smithay.md](docs/spike-smithay.md). anvil (Smithay git main
  `24647fc8d001`) builds in about 2 min and ran nested with its winit backend in KDE (never on a TTY).
  - Works: the shell natively on Wayland (dmabuf, presentation feedback, `--actions`/`--snapshot`),
    X11 clients via anvil's XWayland (`X11Wm`, glxgears ≈ 110 fps, launched by the shell, came to
    the front, shell back after exit), nested gamescope via SDL on Wayland (FSR, 60 fps).
  - Problems: gamescope's native Wayland backend needs `zwp_relative_pointer_manager_v1` (anvil only
    offers it with udev); gamescope's X11/SDL path never showed a window; anvil ignores
    `set_fullscreen` before the first commit; glxgears has no `_NET_WM_PID` (7.3 needs a fallback);
    Smithay has no tearing-control/async page flip yet (VRR is there); Slint has no layer-shell, so
    overlays are shell toplevels placed by the compositor.
  - For the udev backend these packages are missing on this machine: `libinput-devel
    mesa-libgbm-devel libseat-devel libdrm-devel libdisplay-info-devel libei-devel`.
  - Plan for 7.2: `crates/compositor` based on anvil, `--winit`/`--drm`, starts the shell with
    `CONSOLEUI_COMPOSITOR=<control socket>` (line-delimited JSON, types in `consoleui-core`).
    Estimates updated in Epic 7. **Open:** project license (niri/cosmic-comp are GPL).
- **Nested gamescope per entry (7.10):** context menu → "Gamescope options…" opens a card in the
  settings style: start in gamescope on/off, resolution (native/720p/800p/1080p/1440p, width follows
  the output's aspect ratio), upscaler (off/FSR/NIS/integer), FPS limit (off/30/40/60/90), HDR,
  fullscreen; the bottom line previews the prefix. Saved per entry in `game_options.json`. The
  launch runs `gamescope <args> -- <command>` in the same systemd scope, so stopping, exit tracking
  and window ownership work unchanged. Steam games are not supported (Steam starts the game itself;
  the only ways are Steam's own config files) — the card points to `gamescope … -- %command%` in
  Steam's launch options instead. `ENABLE_GAMESCOPE_WSI` is removed for the nested gamescope
  (it showed a "Gamescope WSI Layer Error" inside the session).
  - Tested: unit tests, snapshots, desktop launch with `glxgears` (process tree and scope checked,
    quitting via the quick menu), in the gamescope session the nested window was tagged and in front.
  - After the reboot: in the gamescope session (`gamescope -e`, `--dummy-launch glxgears`, option set
    via `--actions`) the shell started `gamescope -W 960 -H 540 -f -- glxgears`, and gamescope's own
    screenshot shows glxgears in front – the WSI fix works.
  - **Not tested:** HDR, NIS/integer quality, real entries, real gamepad.
- **i18n (6.6):** English is the source language, German comes from one gettext catalog
  `lang/de/LC_MESSAGES/consoleui.po` (≈ 226 messages), bundled into Slint (`@tr`) and embedded in
  `consoleui_core::i18n` for Rust strings (`tr`/`trf`/`trn`) — no runtime gettext, no `.mo` files.
  The language follows `LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG`, or the new settings row
  "Language: Automatic / English / Deutsch", and switches without restart. The on-screen keyboard is
  QWERTZ for German and QWERTY for English; desktop names (`Name[de]`) follow the language. A test
  fails on missing, untranslated or stale catalog entries. Remaining German comments and log
  messages were translated to English.
  - Tested: snapshots of all screens in both languages (software renderer), runtime switch.
  - **Not tested:** overlay/notification windows in gamescope, the real D-Bus backend's error texts,
    real gamepad. Known gaps: entry names loaded at startup keep the old language until restart;
    the German search heading is now "Suchen" (shares the msgid with the △ hint).
- **GPU fault:** a nested gamescope run in the Smithay spike (gamescope over anvil's XWayland)
  triggered NVIDIA `Xid 51 BAD_TSG` at 12:32; afterwards every new Vulkan device failed
  (`NV_ERR_RESET_REQUIRED`) until a reboot. After the reboot both re-runs passed without an Xid: the
  7.10 test in the gamescope session, and nested gamescope via SDL on Wayland and on anvil's XWayland
  on the NVIDIA GPU (details in the spike doc). The earlier fault most likely came from the spike's
  frame-dump patch of anvil. Agents now run GPU tests one after another, not in parallel.
- **Development container:** `dev/toolbox.sh` creates a Fedora toolbox `consoleui-dev` with all
  `-devel` packages (including the Smithay udev backend) and test tools; the code stays on the host.
  Tests, clippy, release build, snapshots and the full anvil build run inside it.

### Sprint 9 – "Own compositor" (done: 7.2, 7.3, test VM, i18n follow-up)

License: the project is now GPL-3.0-or-later (`LICENSE`), so code from niri can be reused.
Three agents worked in parallel (compositor, test VM, i18n), then `cargo fmt` ran over the whole
workspace with a new `rustfmt.toml` (max_width 120, `use_small_heuristics = "Max"`, chosen because it
changed the least code).

- **Compositor (7.2):** new crate `crates/compositor` (binary `consoleui-compositor`, Smithay git
  `24647fc8d001`, built in the dev container). Backends `--winit` (nested) and `--drm` (DRM/KMS +
  libinput + libseat/logind; accepts a software EGL device). All toplevels are fullscreen;
  `set_fullscreen` before the first commit is honoured. Globals: linux-dmabuf v4 with feedback,
  `wl_drm`, drm-syncobj (explicit sync), presentation-time, relative-pointer, pointer-constraints,
  viewporter, single-pixel-buffer, xdg-decoration, data device/primary selection. The compositor starts
  the shell with `CONSOLEUI_COMPOSITOR` and quits with it (SIGTERM/SIGINT/Ctrl+Alt+Backspace clean up
  sockets). `--screenshot-after MS PNG` renders the output offscreen into a PNG (basis for 7.12).
- **Control protocol:** `consoleui_core::protocol`, a Unix socket with one JSON object per line.
  Shell → compositor: `Hello`, `Launched`, `Exited`, `Focus`, `Overlay`, `Screenshot`. Compositor →
  shell: `Welcome`, `Output`, `WindowMapped`, `AppGone`, `FocusChanged`, `ScreenshotDone`. An app comes
  to the front once one of its windows shows content. Shell side: `src/compositor.rs` (takes
  precedence over gamescope; gamescope and desktop mode unchanged).
- **XWayland (7.3):** `X11Wm`; X11 toplevels fullscreen, dialogs centered, override-redirect windows
  above their owner, keyboard focus and X stacking follow the front window, clipboard bridged. Window
  → app mapping via `_NET_WM_PID` or the X-Resource client PID (glxgears has no `_NET_WM_PID`) plus
  `consoleui_core::ownership`; unclaimed windows go to the newest app without a window.
- **Session files:** `session/consoleui-compositor-session` (`--drm`) and
  `session/consoleui-compositor.desktop`; `session/install.sh` installs them when the compositor is
  built; `session/consoleui-nested --compositor` runs the nested session on our compositor.
- **Test VM:** `dev/vm.sh` (plain QEMU/KVM as the user, Fedora 44 Cloud image verified via the signed
  CHECKSUM, cloud-init user `tester`, virtiofs share of the checkout at the same path, `virtio-vga`
  without host GL, `ssh`/`screenshot`/`seat` helpers), see `docs/dev-vm.md`. `seat` runs a command as an
  active logind session on seat0/VT 7.
- **i18n follow-up:** entry names and descriptions follow a runtime language switch (`AppEntry.names`,
  `localize_entries`); the German search heading is "Suche" again (English heading: "Find").
- **Tested:**
  - Nested (`--winit`) on the NVIDIA GPU, one run at a time, no Xid: shell fullscreen, a Wayland app
    and an X11 app (glxgears) through launch → front → quick menu → resume → exit → back to the shell,
    nested gamescope via the 7.10 option with gamescope's own Wayland backend.
  - In the VM with `--drm` on seat0 (llvmpipe): mode 1280×800 set on Virtual-1, the shell rendered,
    glxgears launched through XWayland, mapped to its app and shown fullscreen, focus went to the shell
    for the quick menu and back to the game. ("Unable to become drm master" is logged but harmless.)
  - 128 core + 10 compositor + 8 shell tests, clippy, `cargo fmt --check` (in the dev container).
- **Bug found by hand in the VM:** the keyboard did not reach the shell on `--drm` (the pointer did).
  The shell's first buffer changed no front window, so the policy was not re-applied and the shell
  never got keyboard focus; nested runs hid it because a resize re-applied the policy, and the tests
  used `--actions` instead of real keys. Fixed in `State::window_committed`; verified with keys injected
  through QMP `send-key`. `dev/vm.sh try` now runs compositor + shell in the VM with one command.
- **Not tested:** VT switching and hotplug on DRM, real games, Steam, real gamepad, HiDPI, other
  keyboard layouts, the login session through a display manager.
- **Known gaps (next stories):** toasts are not visible over a running app and the quick menu shows the
  whole shell window above the game (7.6); the game still gets input while the quick menu is open,
  gamepads still use `grab.rs` (7.5); no popup grabs, no cursor on DRM unless the client sets one, no
  `wp_fifo`/commit-timing; background apps get frame callbacks only once a second; unclaimed normal
  windows stay invisible (Steam and launcher windows: 7.4).
- **Workflow change:** the whole workspace (`cargo test/clippy --workspace`) now builds only in the dev
  container, because the compositor needs `-devel` packages; on the host, `cargo build`/`cargo run`
  still build the shell. glxgears is in Fedora's `glx-utils` package (added to the toolbox and VM lists).

### Sprint 10 – "Session without gamescope" (done: 7.5, 7.6, keyboard layout, DRM checks)

Two agents in parallel; both had to test with real key events (QMP `send-key`), not only `--actions`.

- **Input isolation (7.5):** while the quick menu is open, keyboard and pointer go only to shell
  windows; the game's surface gets a pointer leave (its pointer lock/confinement is released and
  restored afterwards). Keyboard focus returns to the game only after all keys are released, so the
  game never sees the release of the key that closed the menu — no `restore_keyboard_focus` hacks.
  F1, Super+Escape and `XF86HomePage` are reserved by the compositor and sent to the shell as `Guide`,
  so the quick menu opens from the keyboard even while a game has focus (found with real keys in the
  VM: before, F1 went to the game). Gamepads: the shell keeps the `EVIOCGRAB` (`grab.rs`, driven by
  the same overlay state) — the shell is the only gamepad reader, and logind revocation would not
  affect fds a game opened itself.
- **Overlays (7.6):** quick menu and toasts are separate shell toplevels announced with
  `ShellMessage::WindowRole`; the policy stacks them above the app (`Layer`), toasts never get input.
  The game stays visible under the menu. Older peers keep working (`Welcome.features`).
- **Keyboard layout:** `consoleui_core::keymap` detects the system layout (here `de (nodeadkeys)`);
  new settings row "Keyboard layout" (only under the compositor, Automatic + 16 layouts), applied at
  runtime via `KeyboardLayout`/`Keymap` messages.
- **DRM checks in the VM (with real keys):** VT switching works (also mid-game, no stuck modifiers);
  a new preferred mode was ignored — fixed; unplugging the output hands over to a second one; login
  through **SDDM** into "ConsoleUI (own compositor)" works and Ctrl+Q returns to the greeter. The
  compositor now logs to stderr and the session to the journal (`journalctl --user -t consoleui`);
  `session/install.sh` accepts a toolbox build. New VM helpers: `dev/vm.sh keys`, `dev/vm.sh output`,
  `VM_HEADS`, a second VM via `CONSOLEUI_VM_DIR`/`VM_SSH_PORT`.
- **Tested:** a foot terminal as stand-in game logged exactly the keys typed outside the menu
  (`abcde`), none of the keys pressed while the menu was open; nested on the NVIDIA host the quick
  menu and a toast showed over glxgears and the game came back (no Xid); gamescope mode regression
  checked; 136 core + 15 compositor + 8 shell tests, clippy, fmt.
- **Not tested:** real gamepad (also the grab in compositor mode), hidraw games and Steam, a real
  game's pointer lock, keyboard isolation of an X11 game with logged keys, password login at the
  greeter (autologin only), GDM/greetd, real monitors (EDID, HDR, refresh rates), HiDPI.
- **Known gaps:** games no longer get F1/Super+Escape/Home under the compositor; hidraw controllers
  still reach the game while the menu is open (possible later fix: `SDL_JOYSTICK_HIDAPI=0`, loses
  gyro/LED/rumble); the overlay stays on top ~450 ms while fading out; a monitor that comes back after
  a hand-over stays dark (single output); shutdown logs a harmless DRM "restore previous state" error.

### Sprint 11 – "Transitions" (done: 7.7, 6.7, 6.8)

- **Transitions (7.7):** `ShellMessage::Focus` carries an optional `Transition` (the tile rectangle
  and how long the shell's launch screen takes to grow); the compositor announces
  `FEATURE_TRANSITIONS`. Opening: the app's windows zoom out of the tile (or out of the launch screen
  where it is at that moment) to fullscreen and fade in; the image keeps its aspect ratio (scaled to
  cover the growing rectangle and cropped to it). Return ("Home" in the quick menu): the app shrinks
  into its tile on the home screen and fades out above the shell, which settles from a slight zoom
  and brightens. If the app closed its window itself, only the shell part runs. The Smithay-free
  logic is in `crates/compositor/src/transition.rs` (unit-tested); `State::frame_windows` builds the
  frame's windows (front to back, the app being left raised above the shell, background apps hidden)
  and `render.rs` turns an `Effect` into rescale/relocate/crop elements. The shell keeps the launch
  screen up ~700 ms under the zooming app. Animations off → no transition (instant, as before).
- **Background music (6.7):** settings row "Background music": Off (default) / Ambient / My music.
  Ambient is generated (`consoleui_core::music`, 48 s loop over four chords with bells and an echo,
  seamless) and streamed to its own `pw-cat --media-role Music` with a 1.5 s fade; "My music" plays
  `flac`/`ogg`/`opus`/`wav` files from `$XDG_DATA_HOME/consoleui/music` shuffled, paused with
  `SIGSTOP`/`SIGCONT`. It plays only while the shell is in front (no app, no launch screen, no boot
  screen). `cargo run -p consoleui-core --example ambient_wav -- /tmp/ambient.wav` writes the loop
  to a WAV file.
- **Animation polish (6.8):** focus ring with a gap around the tile, a light sweeping once across a
  newly focused tile, a ripple ring on press, a glass highlight on every tile, a gliding underline
  under the current tab, the info area on the home screen crossfades (old text sinks, new one rises)
  and an indeterminate progress bar on the launch screen. The settings list scrolls now (nine rows
  with the music row did not fit above the hint bar in a 720p window). Benchmark: render time
  Ø 6.0 → 7.0 ms, max frame gap unchanged.
- **Tested:** nested on the NVIDIA host (`--winit`, glxgears via `--dummy-launch`, one run, no Xid):
  a screenshot series every 70 ms through launch → zoom in, quick menu → Home → shrink into the
  tile, launch again from the running tile → zoom out of the tile. Music with a stand-in `pw-cat`
  that records the stream: fade in after the boot screen, fade out at the launch, back after the app
  exited. 138 core + 21 compositor + 8 shell tests, clippy, fmt.
- **Not tested:** transitions on DRM in the VM, with a real game or Steam (splash windows), with a
  nested gamescope; the music with real speakers over a longer time; "My music" with real files.
- **Known gaps:** no rounded corners or shadow on the zooming game image (would need a shader);
  the return animation needs the app's window, so an app that quits shows only the shell part;
  "My music" pauses without a fade.

### Story 5.7 – Profiles ("Who's playing?")

- **What:** the user asked for a "user selection"; of the two readings (a real greeter/login
  manager for Linux users, or PS5-style profiles inside the shell) they chose the profiles. A greeter
  (greetd IPC or PAM, Linux users with AccountsService avatars) stays open as an idea.
- **Storage:** `profiles.json` (list, last chosen) and `profiles/<id>/` with `history.json`,
  `collection.json`, `game_options.json` and `settings.json` under `$XDG_DATA_HOME/consoleui`. The ID
  is a slug of the first name and stays when renaming. On the first start the profile is named after
  the login name and gets copies of the old files (`~/.config/consoleui/settings.json` included), so
  an older ConsoleUI still works.
- **Flow:** with two or more profiles the shell asks "Who's playing?" after the boot screen (the
  settings of the profile chosen last apply until then, e.g. its language). ✕ chooses, ≡ opens the
  menu (rename, change color, delete with a confirmation; the running profile cannot delete itself),
  "Add profile" opens the name entry (on-screen keyboard with a shift layer, words start with a capital,
  max. 20 characters, at most 8 profiles). The control center has "Switch profile" (also over a running
  game: the game goes to the background, like "Home"). Switching reloads settings (language, accent,
  sounds, music, …), history, pins and game options, re-sorts and greets with a toast; running apps
  keep running. The top bar shows the active profile's avatar instead of the placeholder circle.
- **Code:** `consoleui_core::profiles` and `profile_picker` (UI-free, 15 tests), `src/profiles.rs`
  (start profile, view), `ui/profiles.slint`. The loader no longer reads history/collection/game
  options; the shell applies the active profile's data. Options `--profile <ID|NAME>` and
  `--start profiles`.
- **Tested:** snapshots with a scratch `XDG_DATA_HOME`: the start screen, the menu, the name entry,
  switching at start and back through the control center with the real library (pins, history and
  hidden entries differ per profile, accent and language change), the first start creating a profile
  from the old files, adding and deleting (files removed). 153 core + 8 shell + 21 compositor tests,
  clippy, fmt.
  Also "Switch profile" over a running app on the desktop (glxgears via `--dummy-launch`: it keeps running).
- **Not tested:** with a real gamepad and in the compositor/gamescope session (the screen is a layer
  of the main window, so nothing session-specific is expected).
- **Known gaps:** no avatar images (initial and color only); Steam's own "last played" counts for every
  profile; Steam itself and the games do not know about the profiles (save games are shared); no
  protection (PIN) for a profile.

## Risiken / offene Fragen

- **gamescope-Fokussteuerung** ist kaum dokumentiert und auf XWayland bzw. Steam ausgelegt → deshalb Spike 0.3 früh einplanen. Falls das nicht klappt, wird Epic 7 wichtiger.
- **Overlay über Spielen** (4.6) ist mit gamescope der schwierigste Teil.
- **Testen der Session** am besten zuerst verschachtelt (`gamescope -- consoleui` im KDE-Desktop) und erst danach auf einem TTY bzw. als Login-Session.
