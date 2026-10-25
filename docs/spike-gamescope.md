# Spike 0.3 – Fokussteuerung in gamescope

Getestet mit gamescope 3.16.29 (Fedora 44), verschachtelt im KDE-Plasma-Desktop:
`gamescope -W 1280 -H 720 -- sh -c 'consoleui --dummy & …; glxgears & …'`.

## Ergebnisse

1. **Umgebung der Kindprozesse:** `DISPLAY=:1`, `WAYLAND_DISPLAY` ist leer,
   `GAMESCOPE_WAYLAND_DISPLAY=gamescope-0`. Die Shell (Slint/winit) läuft deshalb über XWayland
   in gamescope, ebenso alles, was sie startet. Wir brauchen kein `--expose-wayland`.
2. **Fokus ohne `-e` (keine Steam-Integration):** Das zuletzt geöffnete Fenster bekommt den Fokus
   und wird angezeigt. Schließt es sich, ist automatisch wieder die Shell zu sehen. Getestet:
   Shell → `glxgears` startet (Spiel sichtbar) → `glxgears` beendet (Shell wieder sichtbar).
3. **Mit `-e`** erwartet gamescope die Steam-Atome (`STEAM_GAME`, `GAMESCOPECTRL_BASELAYER_APPID`),
   die Steam selbst setzt. Für unsere Shell lassen wir `-e` weg. Erst wenn wir Overlays über
   Spielen brauchen (Story 4.6), schauen wir uns die Atome genauer an.

## Folgen für die Umsetzung

- **4.1/4.2:** Die Session ist einfach `gamescope [Optionen] -- consoleui`, ohne `-e`.
- **4.3:** Das Fenster wechseln wir nicht selbst, das macht gamescope. Die Shell muss nur merken,
  wann die App beendet ist, um den Startbildschirm zurückzufahren:
  - Desktop-Apps: Der gestartete Prozess endet.
  - Steam-Spiele: Der Prozess `steam steam://rungameid/…` kehrt sofort zurück. Ob das Spiel läuft,
    steht in `~/.steam/registry.vdf` unter `HKCU/Software/Valve/Steam/RunningAppID`. (Später ersetzt: Steam aktualisiert den Wert erst Minuten nach dem Spielende; die Shell sucht jetzt nach Prozessen mit `SteamAppId`.)
- **Wichtig für Steam:** Läuft Steam schon außerhalb von gamescope (z. B. im KDE-Desktop), öffnet
  `steam://` das Spiel dort und nicht in gamescope. In der echten Session (TTY/Login-Manager) startet
  Steam beim ersten `steam://`-Aufruf innerhalb der Session, das passt dann.
- **Offen (4.6):** Ein Overlay über laufenden Spielen geht ohne `-e`/Atome nicht, weil gamescope
  nur das Fenster mit Fokus zeigt.

# Spike 2 (Sprint 5) – Zurückwechseln und Overlay

Getestet verschachtelt mit gamescope 3.16.29, mit und ohne `-e`. Die Screenshots kamen über
`GAMESCOPECTRL_REQUEST_SCREENSHOT` (landet in `/tmp/gamescope.png`). „Spiel“ war `glxgears` bzw.
eine zweite Shell-Instanz.

| Versuch | ohne `-e` | mit `-e` |
|---|---|---|
| `GAMESCOPECTRL_BASELAYER_WINDOW` = Shell-Fenster | wird ignoriert | – |
| Shell verstecken und wieder anzeigen (neu mappen) | holt die Shell **nicht** nach vorn | – |
| Fenster ohne `STEAM_GAME`-Atom | fokussierbar (neuestes gewinnt) | **nie** fokussierbar, Bild bleibt leer |
| `STEAM_GAME` an Shell (769) und Spiel (4242), `BASELAYER_APPID` = `769` | – | **Shell vorn**, Spiel läuft weiter |
| `BASELAYER_APPID` = `4242,769` | – | **Spiel vorn** |
| `SteamGameId` nur in der Umgebung (Fenster mit `_NET_WM_PID`) | – | wird **nicht** übernommen |
| `STEAM_OVERLAY` an deckender Shell (nachträglich oder vor dem Remap) | nicht über dem Spiel zu sehen | nicht über dem Spiel zu sehen |

**Folgerungen**

- Gezielt umschalten (Story 4.4, 3.4) geht nur im Steam-Modus (`-e`). Dann muss die Shell
  die Rolle von Steam übernehmen: Sie setzt `STEAM_GAME` an alle Fenster (die eigenen und die
  der gestarteten Apps, zuordnen lässt sich das über `_NET_WM_PID` → Prozess/cgroup bzw.
  `SteamAppId` in der Umgebung) und steuert mit `GAMESCOPECTRL_BASELAYER_APPID`, wer vorne ist.
  Fenster ohne Atom bleiben sonst unsichtbar.
- Ein echtes Overlay über dem laufenden Spiel (4.6) braucht mehr. Vermutlich muss es ein eigenes,
  transparentes Fenster sein, bei dem `STEAM_OVERLAY` schon vor dem ersten Mappen gesetzt ist.
  Das ist noch nicht getestet. Die Alternative: Das Quick-Menü ist ein Bildschirm der Shell, der
  per Baselayer nach vorn kommt (das Spiel läuft im Hintergrund weiter).

# Spike 3 (Sprint 5) – Echtes Overlay

- Ein Fenster mit 32-Bit-Visual (ARGB), bei dem `STEAM_OVERLAY=1` **vor dem Mappen** gesetzt ist
  und das so breit wie das Root-Fenster ist, legt gamescope halbtransparent über das Spiel
  (`examples/overlay_spike.rs`). gamescope liest die Atome in `map_win`, `_NET_WM_WINDOW_OPACITY`
  ist standardmäßig `OPAQUE`, und `STEAM_INPUT_FOCUS=1` gibt Tastatur und Maus an das Overlay.
- **Achtung beim Testen:** Der Wert von `GAMESCOPECTRL_REQUEST_SCREENSHOT` ist der
  Screenshot-Typ. `1` nimmt nur die unterste Ebene auf (ohne Overlay), `2` die volle Komposition.
  Mit `1` sah es zunächst so aus, als ginge das Overlay nicht.
- `SteamGameId` in der Umgebung reicht nicht: Fenster brauchen das `STEAM_GAME`-Atom, sonst sind
  sie im Steam-Modus unsichtbar. Die Shell ordnet sie über `_NET_WM_PID` zu (siehe
  `consoleui_core::ownership`).
- Ende-zu-Ende verschachtelt getestet (`--dummy --dummy-launch glxgears --actions …`): Starten,
  Overlay über dem Spiel, Home mit laufendem Spiel, Fortsetzen, Beenden mit Bestätigung.

# Spike 4 (Sprint 7) – Benachrichtigungsfenster

Für Toasts über laufenden Spielen. Quellen: `steamcompmgr.cpp` aus gamescope (3.16.29), getestet
verschachtelt mit `-e -W 960 -H 540` und `glxgears`.

- `DetermineAndApplyFocus`: Ein `STEAM_OVERLAY`-Fenster, das so breit wie das Root-Fenster ist
  oder `STEAM_INPUT_FOCUS` setzt, wird das interaktive Overlay. Jedes andere wird das
  `notificationWindow`. Fenster mit `STEAM_OVERLAY` kommen nie in die Liste der fokussierbaren
  Fenster. Das Benachrichtigungsfenster nimmt dem Spiel also weder Tastatur noch Maus weg.
  (In älteren Versionen, z. B. 3.16.20, zählt nur `nWidth > 1200`.)
- `paint_all` zeichnet das Benachrichtigungsfenster immer als eigene Ebene über Spiel und Overlay
  (wenn `opacity` ≠ 0). Die **Position wird ignoriert**: `paint_window_commit` skaliert das
  Fenster auf die Ausgabe (wie das Spiel) und zentriert es. Ein kleines Fenster würde also
  vergrößert in der Mitte erscheinen. Deshalb ist das Fenster genau 1 px schmaler als das Root-
  Fenster und so hoch wie der Bildschirm (Maßstab ≈ 1), transparent, und der Toast liegt darin oben
  rechts.
- `handle_desktop_window` zieht Fenster ohne `STEAM_GAME` auf Root-Größe, außer sie haben feste
  Size-Hints (min = max). Ohne die war das Fenster wieder 960 breit und damit ein Overlay. In Slint
  reicht es, `width`/`height` des Fensters fest zu binden (Rust setzt sie aus der Root-Geometrie).
- Ergebnis: Hinweis über `glxgears` zu sehen, `GAMESCOPE_FOCUSED_WINDOW` und `XGetInputFocus`
  bleiben beim Spiel. Mit offener Steuerzentrale sind beide zu sehen, und den Fokus hat das Overlay.
- Nebenbefund (nicht neu): Nach dem Schließen der Steuerzentrale steht `GAMESCOPE_FOCUSED_APP`
  leer und `XGetInputFocus` zeigt auf ein gamescope-internes Fenster statt aufs Spiel. Das war auch
  ohne Hinweisfenster so.

