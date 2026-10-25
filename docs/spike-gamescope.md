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
    steht in `~/.steam/registry.vdf` unter `HKCU/Software/Valve/Steam/RunningAppID`.
- **Wichtig für Steam:** Läuft Steam schon außerhalb von gamescope (z. B. im KDE-Desktop), öffnet
  `steam://` das Spiel dort und nicht in gamescope. In der echten Session (TTY/Login-Manager) startet
  Steam beim ersten `steam://`-Aufruf innerhalb der Session, das passt dann.
- **Offen (4.6):** Ein Overlay über laufenden Spielen geht ohne `-e`/Atome nicht, weil gamescope
  nur das Fenster mit Fokus zeigt.
