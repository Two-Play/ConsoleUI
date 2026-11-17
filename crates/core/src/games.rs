//! Windows games outside Steam (Epic 10, Story 10.2): the model of the game manager `consoleui-games`.
//!
//! Every game lives in its own folder `$XDG_DATA_HOME/consoleui/games/<id>/` with `game.json` (a
//! [`GameInstall`]) and, unless the user chose another one, its Wine prefix `prefix/`. Games run with Proton
//! outside Steam through `umu-run` (umu-launcher, see `docs/spike-umu.md`). The library sees them through a
//! desktop entry `consoleui-game-<id>.desktop` that the app writes ([`GameInstall::desktop_entry`]); it runs
//! `consoleui-games --run <id>`, which replaces itself with `umu-run` ([`GameInstall::launch_plan`]), so the
//! shell's scope tracks the game like any other app.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search;
use crate::xdg;

/// umu's program
pub const UMU_RUN: &str = "umu-run";
/// Prefix of the desktop entries the game manager writes; the rest is the game's ID
pub const DESKTOP_PREFIX: &str = "consoleui-game-";
/// Desktop entry key that names the game an entry belongs to
pub const DESKTOP_KEY: &str = "X-ConsoleUI-Game";
/// `GAMEID` for games the umu database does not know: no game-specific fixes
pub const UMU_DEFAULT: &str = "umu-default";

/// Which Proton runs a game.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "value")]
pub enum Proton {
    /// umu's choice: the newest UMU-Proton, downloaded by umu itself (`PROTONPATH` unset)
    #[default]
    Umu,
    /// The newest GE-Proton, downloaded by umu (`PROTONPATH=GE-Proton`)
    GeLatest,
    /// An installed build: its folder (with the `proton` script inside)
    Path(PathBuf),
}

impl Proton {
    /// Value for `PROTONPATH`, `None` to let umu choose.
    pub fn protonpath(&self) -> Option<String> {
        match self {
            Self::Umu => None,
            Self::GeLatest => Some("GE-Proton".into()),
            Self::Path(path) => Some(path.to_string_lossy().into_owned()),
        }
    }
}

/// One installed game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameInstall {
    /// File-name safe, unique among the games ([`unique_id`]); also part of the desktop entry's name
    pub id: String,
    pub title: String,
    /// The game's Windows executable (absolute, on the Linux side)
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    /// Working directory; `None` is the executable's folder, as most games expect
    pub working_dir: Option<PathBuf>,
    /// Wine prefix; `None` is `prefix/` in the game's own folder
    pub prefix: Option<PathBuf>,
    pub proton: Proton,
    /// umu's `GAMEID` (e.g. `umu-1091500`) when the umu database knows the game; brings its fixes
    pub umu_id: Option<String>,
    /// umu's `STORE` (`gog`, `egs`, …) for the database lookup
    pub store: Option<String>,
    /// Direct3D through WineD3D (OpenGL) instead of DXVK/VKD3D, for games those break
    pub wined3d: bool,
    /// More environment variables, set last (they win)
    pub environment: BTreeMap<String, String>,
    /// Artwork as absolute paths (or icon names), for the desktop entry's `X-ConsoleUI-*` keys
    pub cover: Option<String>,
    pub hero: Option<String>,
    pub logo: Option<String>,
}

impl Default for GameInstall {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            executable: PathBuf::new(),
            arguments: Vec::new(),
            working_dir: None,
            prefix: None,
            proton: Proton::default(),
            umu_id: None,
            store: None,
            wined3d: false,
            environment: BTreeMap::new(),
            cover: None,
            hero: None,
            logo: None,
        }
    }
}

/// What `consoleui-games --run` executes: program, arguments, working directory and environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    /// Set in this order (later ones win)
    pub env: Vec<(String, String)>,
}

impl GameInstall {
    /// The Wine prefix: the chosen one or `prefix/` in `game_dir`.
    pub fn prefix_in(&self, game_dir: &Path) -> PathBuf {
        self.prefix.clone().unwrap_or_else(|| game_dir.join("prefix"))
    }

    /// The command that runs the game: `umu-run <executable> <arguments>` with the prefix, Proton and the
    /// umu database ID in the environment. `game_dir` is the game's folder, `umu_run` the program found by
    /// [`find_umu_run`].
    pub fn launch_plan(&self, game_dir: &Path, umu_run: &Path) -> LaunchPlan {
        let working_dir = self
            .working_dir
            .clone()
            .or_else(|| self.executable.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| game_dir.to_path_buf());
        let mut env = vec![
            ("WINEPREFIX".to_string(), self.prefix_in(game_dir).to_string_lossy().into_owned()),
            ("GAMEID".to_string(), self.umu_id.clone().unwrap_or_else(|| UMU_DEFAULT.into())),
        ];
        if let Some(path) = self.proton.protonpath() {
            env.push(("PROTONPATH".into(), path));
        }
        if let Some(store) = &self.store {
            env.push(("STORE".into(), store.clone()));
        }
        if self.wined3d {
            env.push(("PROTON_USE_WINED3D".into(), "1".into()));
        }
        env.extend(self.environment.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut args = vec![self.executable.to_string_lossy().into_owned()];
        args.extend(self.arguments.iter().cloned());
        LaunchPlan { program: umu_run.to_path_buf(), args, working_dir, env }
    }

    /// File name of the game's desktop entry, e.g. `consoleui-game-far-cry-6.desktop`.
    pub fn desktop_file_name(&self) -> String {
        format!("{DESKTOP_PREFIX}{}.desktop", self.id)
    }

    /// The desktop entry that puts the game into the shell's library (and desktop menus). `runner` is the
    /// game manager's program (`consoleui-games`, or its absolute path).
    pub fn desktop_entry(&self, runner: &str) -> String {
        let mut text = String::from("[Desktop Entry]\nType=Application\n");
        text += &format!("Name={}\n", escape_value(&self.title));
        let exec = [runner, "--run", &self.id].map(exec_quote).join(" ");
        text += &format!("Exec={}\n", escape_value(&exec));
        text += "Categories=Game;\nX-ConsoleUI=true\nX-ConsoleUI-Kind=game\n";
        text += &format!("{DESKTOP_KEY}={}\n", escape_value(&self.id));
        for (key, value) in [("Icon", &self.cover), ("X-ConsoleUI-Cover", &self.cover)]
            .into_iter()
            .chain([("X-ConsoleUI-Hero", &self.hero), ("X-ConsoleUI-Logo", &self.logo)])
        {
            if let Some(value) = value {
                text += &format!("{key}={}\n", escape_value(value));
            }
        }
        text
    }
}

/// `$XDG_DATA_HOME/consoleui/games`
pub fn default_dir() -> Option<PathBuf> {
    xdg::data_home().map(|dir| dir.join("consoleui/games"))
}

/// `$XDG_DATA_HOME/applications`, where the desktop entries go
pub fn default_entries_dir() -> Option<PathBuf> {
    xdg::data_home().map(|dir| dir.join("applications"))
}

/// All games in `dir`, sorted by title. Folders without a valid `game.json` are skipped with a log line.
pub fn load_all(dir: &Path) -> Vec<GameInstall> {
    let mut games: Vec<GameInstall> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path().join("game.json");
            let text = fs::read_to_string(&path).ok()?;
            match serde_json::from_str::<GameInstall>(&text) {
                // The folder name is the ID; a copied folder must not show up twice under one ID
                Ok(game) if entry.file_name().to_str() == Some(game.id.as_str()) => Some(game),
                Ok(_) => {
                    eprintln!("Game {}: the ID does not match its folder, skipped", path.display());
                    None
                }
                Err(err) => {
                    eprintln!("Game {} is invalid and skipped: {err}", path.display());
                    None
                }
            }
        })
        .collect();
    games.sort_by(|a, b| search::normalize(&a.title).cmp(&search::normalize(&b.title)).then(a.id.cmp(&b.id)));
    games
}

/// Writes `<dir>/<id>/game.json` (through a temporary file, so a crash cannot destroy it).
pub fn save(dir: &Path, game: &GameInstall) -> io::Result<()> {
    if !is_valid_id(&game.id) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid game ID {:?}", game.id)));
    }
    let game_dir = dir.join(&game.id);
    fs::create_dir_all(&game_dir)?;
    let path = game_dir.join("game.json");
    let tmp = game_dir.join("game.json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(game)?)?;
    fs::rename(tmp, path)
}

/// Writes the game's desktop entry into `entries_dir`.
pub fn write_desktop_entry(entries_dir: &Path, game: &GameInstall, runner: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(entries_dir)?;
    let path = entries_dir.join(game.desktop_file_name());
    let tmp = path.with_extension("desktop.tmp");
    fs::write(&tmp, game.desktop_entry(runner))?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// IDs are lowercase ASCII letters, digits and single dashes inside, like `far-cry-6`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
}

/// An ID from the title (`Far Cry® 6` → `far-cry-6`), with `-2`, `-3`, … when `taken` already has it.
pub fn unique_id(title: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut base = String::new();
    for c in search::normalize(title).chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let mut base = base.trim_end_matches('-').to_string();
    if base.len() > 56 {
        base.truncate(56);
        base = base.trim_end_matches('-').to_string();
    }
    if base.is_empty() {
        base = "game".into();
    }
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).expect("some suffix is free")
}

/// Where `umu-run` is: in `PATH` (a distribution package), then the copy the game manager downloaded
/// (`$XDG_DATA_HOME/consoleui/umu/umu-run`), then the one Lutris keeps. `exists` checks a candidate.
pub fn find_umu_run(path_var: &str, data_home: Option<&Path>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let in_path = path_var.split(':').filter(|dir| !dir.is_empty()).map(|dir| Path::new(dir).join(UMU_RUN));
    let own = data_home
        .into_iter()
        .flat_map(|data| [data.join("consoleui/umu").join(UMU_RUN), data.join("lutris/runtime/umu").join(UMU_RUN)]);
    in_path.chain(own).find(|candidate| exists(candidate))
}

/// A Proton build found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtonBuild {
    /// Its folder name, e.g. `GE-Proton11-12-x86_64`
    pub name: String,
    pub path: PathBuf,
}

/// Proton builds in the given `compatibilitytools.d` folders (Steam's, umu's downloads land there too),
/// newest name first; the same name in two folders counts once (the first folder wins).
pub fn installed_protons(dirs: &[PathBuf]) -> Vec<ProtonBuild> {
    let mut builds: Vec<ProtonBuild> = Vec::new();
    for dir in dirs {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
            if path.join("proton").is_file() && !builds.iter().any(|b| b.name == name) {
                builds.push(ProtonBuild { name, path });
            }
        }
    }
    builds.sort_by(|a, b| natural_cmp(&b.name, &a.name));
    builds
}

/// The usual places of `compatibilitytools.d`: Steam's (native and `~/.steam/root`) and the Flatpak Steam's.
pub fn compat_tool_dirs(home: &Path, data_home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        data_home.join("Steam/compatibilitytools.d"),
        home.join(".steam/root/compatibilitytools.d"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/compatibilitytools.d"),
    ];
    dirs.dedup();
    dirs
}

/// Compares names with their numbers as numbers, so `GE-Proton10-9` < `GE-Proton10-10`; names that differ
/// only in leading zeros fall back to plain text order.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    natural_cmp_bytes(a.as_bytes(), b.as_bytes()).then_with(|| a.cmp(b))
}

fn natural_cmp_bytes(mut a: &[u8], mut b: &[u8]) -> std::cmp::Ordering {
    loop {
        match (a.first(), b.first()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let len_a = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let len_b = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (num_a, num_b) = (trim_zeros(&a[..len_a]), trim_zeros(&b[..len_b]));
                let order = num_a.len().cmp(&num_b.len()).then_with(|| num_a.cmp(num_b));
                if order.is_ne() {
                    return order;
                }
                (a, b) = (&a[len_a..], &b[len_b..]);
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                (a, b) = (&a[1..], &b[1..]);
            }
        }
    }
}

fn trim_zeros(digits: &[u8]) -> &[u8] {
    let start = digits.iter().position(|&d| d != b'0').unwrap_or(digits.len());
    &digits[start..]
}

/// Escapes a desktop entry string value (`\\`, `\n`, `\t`, `\r`).
fn escape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// Quotes one `Exec` argument when it needs it (the desktop entry spec's reserved characters); `%` is
/// doubled, since a single one starts a field code.
fn exec_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let plain = !arg.is_empty()
        && arg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '+' | ',' | ':' | '='));
    if plain {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop;

    fn game() -> GameInstall {
        GameInstall {
            id: "far-cry-6".into(),
            title: "Far Cry® 6".into(),
            executable: "/games/Far Cry 6/bin/FarCry6.exe".into(),
            arguments: vec!["-skipintro".into()],
            ..GameInstall::default()
        }
    }

    #[test]
    fn launch_plan_runs_umu_in_the_games_folder() {
        let dir = Path::new("/data/consoleui/games/far-cry-6");
        let plan = game().launch_plan(dir, Path::new("/usr/bin/umu-run"));
        assert_eq!(plan.program, Path::new("/usr/bin/umu-run"));
        assert_eq!(plan.args, ["/games/Far Cry 6/bin/FarCry6.exe", "-skipintro"]);
        assert_eq!(plan.working_dir, Path::new("/games/Far Cry 6/bin"));
        assert_eq!(
            plan.env,
            [
                ("WINEPREFIX".to_string(), "/data/consoleui/games/far-cry-6/prefix".to_string()),
                ("GAMEID".to_string(), "umu-default".to_string()),
            ]
        );

        let custom = GameInstall {
            working_dir: Some("/games/Far Cry 6".into()),
            prefix: Some("/pfx".into()),
            proton: Proton::Path("/steam/compatibilitytools.d/GE-Proton11-12".into()),
            umu_id: Some("umu-2369390".into()),
            store: Some("egs".into()),
            wined3d: true,
            environment: BTreeMap::from([("DXVK_HUD".into(), "fps".into()), ("GAMEID".into(), "umu-x".into())]),
            ..game()
        };
        let plan = custom.launch_plan(dir, Path::new("umu-run"));
        assert_eq!(plan.working_dir, Path::new("/games/Far Cry 6"));
        let env: Vec<String> = plan.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(
            env,
            [
                "WINEPREFIX=/pfx",
                "GAMEID=umu-2369390",
                "PROTONPATH=/steam/compatibilitytools.d/GE-Proton11-12",
                "STORE=egs",
                "PROTON_USE_WINED3D=1",
                "DXVK_HUD=fps",
                // The user's own variables come last and win
                "GAMEID=umu-x",
            ]
        );
        assert_eq!(Proton::GeLatest.protonpath().as_deref(), Some("GE-Proton"));
        assert_eq!(Proton::Umu.protonpath(), None);
    }

    #[test]
    fn desktop_entry_round_trips_through_the_shells_parser() {
        let mut game = game();
        game.cover = Some("/data/covers/far cry.png".into());
        game.logo = Some("far-cry-logo".into());
        let text = game.desktop_entry("/opt/Console UI/consoleui-games");
        let entry = desktop::parse(&game.desktop_file_name(), &text).expect("valid entry");
        assert_eq!(entry.id, "consoleui-game-far-cry-6.desktop");
        assert!(entry.console_app);
        assert_eq!(entry.kind, Some(crate::AppKind::Game));
        assert_eq!(entry.cover.as_deref(), Some("/data/covers/far cry.png"));
        assert_eq!(entry.hero, None);
        assert_eq!(entry.logo.as_deref(), Some("far-cry-logo"));
        assert_eq!(
            desktop::exec_to_argv(entry.exec.as_deref().unwrap()),
            ["/opt/Console UI/consoleui-games", "--run", "far-cry-6"]
        );
        assert!(text.contains("Name=Far Cry® 6\n") && text.contains("X-ConsoleUI-Game=far-cry-6\n"));
    }

    #[test]
    fn exec_arguments_are_quoted_and_escaped() {
        assert_eq!(exec_quote("consoleui-games"), "consoleui-games");
        assert_eq!(exec_quote("/a b/c"), "\"/a b/c\"");
        assert_eq!(exec_quote("100%"), "\"100%%\"");
        assert_eq!(exec_quote("a\"$`\\b"), "\"a\\\"\\$\\`\\\\b\"");
        assert_eq!(exec_quote(""), "\"\"");
        assert_eq!(escape_value("a\\b\nc"), "a\\\\b\\nc");
        // What the shell reads back
        let exec = ["/x/it's \"odd\"", "50%"].map(exec_quote).join(" ");
        let text = format!("[Desktop Entry]\nType=Application\nName=X\nExec={}\n", escape_value(&exec));
        let entry = desktop::parse("x.desktop", &text).unwrap();
        assert_eq!(desktop::exec_to_argv(entry.exec.as_deref().unwrap()), ["/x/it's \"odd\"", "50%"]);
    }

    #[test]
    fn ids_from_titles() {
        let none = |_: &str| false;
        assert_eq!(unique_id("Far Cry® 6", none), "far-cry-6");
        assert_eq!(unique_id("  Grüße aus Köln!  ", none), "grusse-aus-koln");
        assert_eq!(unique_id("ファイナル", none), "game");
        assert_eq!(unique_id("", none), "game");
        let taken = |id: &str| id == "doom" || id == "doom-2";
        assert_eq!(unique_id("DOOM", taken), "doom-3");
        let long = unique_id(&"word ".repeat(30), none);
        assert!(long.len() <= 56 && is_valid_id(&long), "{long}");
        assert!(is_valid_id("far-cry-6"));
        for bad in ["", "-a", "a-", "a--b", "A", "a b", "../x", "a/b"] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn finds_umu_run() {
        let data = Path::new("/home/u/.local/share");
        let found = |paths: &'static [&'static str]| move |p: &Path| paths.iter().any(|x| Path::new(x) == p);
        assert_eq!(
            find_umu_run(
                "/bin::/usr/bin",
                Some(data),
                found(&["/usr/bin/umu-run", "/home/u/.local/share/consoleui/umu/umu-run"])
            ),
            Some("/usr/bin/umu-run".into())
        );
        assert_eq!(
            find_umu_run("/bin", Some(data), found(&["/home/u/.local/share/lutris/runtime/umu/umu-run"])),
            Some("/home/u/.local/share/lutris/runtime/umu/umu-run".into())
        );
        assert_eq!(find_umu_run("", None, found(&[])), None);
    }

    #[test]
    fn natural_order_of_proton_names() {
        let mut names = ["GE-Proton10-9", "GE-Proton10-10", "GE-Proton9-27", "UMU-Proton-10.0-4", "GE-Proton10-010"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["GE-Proton9-27", "GE-Proton10-9", "GE-Proton10-010", "GE-Proton10-10", "UMU-Proton-10.0-4"]);
    }

    #[test]
    fn stores_games_and_finds_protons() {
        let root = std::env::temp_dir().join(format!("consoleui-games-{}", std::process::id()));
        let dir = root.join("games");
        let mut doom = GameInstall { id: "doom".into(), title: "DOOM".into(), ..GameInstall::default() };
        save(&dir, &game()).unwrap();
        save(&dir, &doom).unwrap();
        assert!(save(&dir, &GameInstall { id: "../evil".into(), ..GameInstall::default() }).is_err());
        // A broken file and a folder whose name is not the game's ID are skipped
        fs::create_dir_all(dir.join("broken")).unwrap();
        fs::write(dir.join("broken/game.json"), "{").unwrap();
        fs::create_dir_all(dir.join("copy")).unwrap();
        fs::copy(dir.join("doom/game.json"), dir.join("copy/game.json")).unwrap();
        let games = load_all(&dir);
        assert_eq!(games.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["doom", "far-cry-6"]);
        assert_eq!(games[1], game());
        doom.wined3d = true;
        save(&dir, &doom).unwrap();
        assert!(load_all(&dir)[0].wined3d);
        assert_eq!(doom.prefix_in(&dir.join("doom")), dir.join("doom/prefix"));

        let entries = root.join("applications");
        let path = write_desktop_entry(&entries, &doom, "consoleui-games").unwrap();
        assert_eq!(path, entries.join("consoleui-game-doom.desktop"));
        assert!(fs::read_to_string(path).unwrap().contains("Exec=consoleui-games --run doom\n"));

        let (a, b) = (root.join("steam"), root.join("other"));
        for (base, name) in
            [(&a, "GE-Proton10-9"), (&a, "UMU-Proton-10.0-4"), (&b, "GE-Proton10-10"), (&b, "GE-Proton10-9")]
        {
            fs::create_dir_all(base.join(name)).unwrap();
            fs::write(base.join(name).join("proton"), "").unwrap();
        }
        fs::create_dir_all(a.join("not-a-proton")).unwrap();
        let protons = installed_protons(&[a.clone(), b, root.join("missing")]);
        assert_eq!(
            protons.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["UMU-Proton-10.0-4", "GE-Proton10-10", "GE-Proton10-9"]
        );
        assert_eq!(protons[2].path, a.join("GE-Proton10-9"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn json_keeps_working_with_missing_fields() {
        let game: GameInstall =
            serde_json::from_str(r#"{"id": "x", "title": "X", "proton": {"kind": "path", "value": "/p"}}"#).unwrap();
        assert_eq!(game.proton, Proton::Path("/p".into()));
        assert!(!game.wined3d && game.arguments.is_empty());
        let game: GameInstall = serde_json::from_str(r#"{"id": "x", "proton": {"kind": "gelatest"}}"#).unwrap();
        assert_eq!(game.proton, Proton::GeLatest);
    }
}
