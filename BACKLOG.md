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
Smithay-Compositor. Weil die Shell vom Compositor getrennt ist, bleibt der Code der Shell
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

## Epic 5 – System & Einstellungen

| ID | User Story | SP |
|---|---|---|
| 5.1 | Power-Menü: Ausschalten, Neustart, Standby (logind via `zbus`) | 3 |
| 5.2 | Lautstärke einstellen (PipeWire) | 3 |
| 5.3 | WLAN-Liste und Verbinden (NetworkManager via D-Bus) | 8 |
| 5.4 | Bluetooth-Controller koppeln (BlueZ via D-Bus) | 8 |
| 5.5 | Einstellungsseite: Theme/Akzentfarbe, Sounds, Auflösung | 5 |
| 5.6 | Benachrichtigungen als Toasts (Download fertig, Controller verbunden) | 3 |

## Epic 6 – Polish & Animationen

| ID | User Story | SP |
|---|---|---|
| 6.1 | Animierter Boot-/Startbildschirm mit Logo | 2 |
| 6.2 | Übergänge zwischen Seiten (Home ↔ Bibliothek ↔ Einstellungen) mit Slide/Fade | 3 |
| 6.3 | Parallax bzw. leichte Bewegung im Hintergrund-Artwork | 3 |
| 6.4 | Theme-System mit zentralen Design-Tokens (Farben, Radien, Timings) | 3 |
| 6.5 | Performance-Pass: Bilder asynchron laden, Thumbnail-Cache, Framezeit-Overlay zum Debuggen | 5 |

## Epic 7 (optional, Phase 3) – Eigener Smithay-Compositor

| ID | Story | SP |
|---|---|---|
| 7.1 | Spike: Smithay-Beispiel `anvil` bauen und auf einem TTY starten | 5 |
| 7.2 | Minimaler Compositor: DRM/KMS + libinput, ein Vollbild-Client | 13 |
| 7.3 | XWayland-Support (für Steam und die meisten Spiele nötig) | 13 |
| 7.4 | Animierte Übergänge zwischen Shell und App direkt im Compositor | 8 |
| 7.5 | Shell als Layer-Shell-Overlay über laufenden Apps | 8 |

---

## Sprint-Plan (Vorschlag)

| Sprint | Ziel | Stories |
|---|---|---|
| **1** | „Es bewegt sich“: Home-Screen mit Dummy-Daten und flüssiger Navigation | 0.1, 0.2, 0.4, 1.1, 1.2, 3.2 |
| **2** | „Echte Apps“: Desktop- und Steam-Apps erkennen, mit Covern anzeigen und starten | 2.1, 2.2, 2.3, 1.3, 1.5, 2.6 |
| **3** | „Couch-tauglich“: Gamepad-Steuerung und Konsolen-Session mit gamescope | 3.1, 0.3, 4.1, 4.2, 4.3, 1.4 |
| **4** | „Bibliothek“: Grid, Suche, laufende Apps | 2.4, 2.5, 4.4, 4.5, 3.3 |
| **5** | „System“: Power, Lautstärke, Quick-Menü | 5.1, 5.2, 3.4, 4.6 |
| **6** | „Polish“: Animationen, Einstellungen, Performance | 6.x, 5.5, 5.6, 1.6 |
| 7+ | WLAN/Bluetooth, danach entscheiden: Smithay-Compositor ja/nein | 5.3, 5.4, Epic 7 |

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
  genutzt / A–Z, Umlaute richtig einsortiert).
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

## Risiken / offene Fragen

- **gamescope-Fokussteuerung** ist kaum dokumentiert und auf XWayland bzw. Steam ausgelegt → deshalb Spike 0.3 früh einplanen. Falls das nicht klappt, wird Epic 7 wichtiger.
- **Overlay über Spielen** (4.6) ist mit gamescope der schwierigste Teil.
- **Testen der Session** am besten zuerst verschachtelt (`gamescope -- consoleui` im KDE-Desktop) und erst danach auf einem TTY bzw. als Login-Session.
