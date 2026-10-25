//! Which launched app does a process (and thus its window) belong to? In Steam mode gamescope
//! only shows windows with a `STEAM_GAME` atom, so the shell has to assign every window.

use std::fs;

/// gamescope app ID of the shell itself. Far above all Steam app IDs.
pub const SHELL_APP_ID: u32 = 0x7fff_0000;
/// First app ID for launched desktop apps (assigned sequentially)
pub const FIRST_DESKTOP_APP_ID: u32 = SHELL_APP_ID + 1;

/// How the processes of a launched app are recognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// Steam sets `SteamAppId` in the environment of every game process.
    Steam(u32),
    /// Desktop app in a systemd scope unit, e.g. `consoleui-app-123.scope`
    Scope(String),
    /// Desktop app without a scope: the launched process and its descendants
    Process(u32),
}

/// What we need to know about a process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessInfo {
    pub parent: Option<u32>,
    pub steam_app_id: Option<u32>,
    pub cgroup: String,
}

pub fn read_process(pid: u32) -> Option<ProcessInfo> {
    let dir = format!("/proc/{pid}");
    let stat = fs::read_to_string(format!("{dir}/stat")).ok()?;
    let environ = fs::read(format!("{dir}/environ")).unwrap_or_default();
    Some(ProcessInfo {
        parent: parent_from_stat(&stat),
        steam_app_id: steam_app_id(&environ),
        cgroup: fs::read_to_string(format!("{dir}/cgroup")).unwrap_or_default(),
    })
}

/// Fourth field of `/proc/<pid>/stat`; the process name in parentheses may contain spaces.
fn parent_from_stat(stat: &str) -> Option<u32> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    after_name.split_whitespace().nth(1)?.parse().ok().filter(|&p| p > 1)
}

fn steam_app_id(environ: &[u8]) -> Option<u32> {
    environ
        .split(|&b| b == 0)
        .find_map(|var| var.strip_prefix(b"SteamAppId="))
        .and_then(|v| std::str::from_utf8(v).ok()?.parse().ok())
        .filter(|&id| id != 0)
}

/// Walks up the parent processes to find the app `pid` belongs to; `apps` are (app ID, matcher).
pub fn owner_of(pid: u32, apps: &[(u32, Owner)], lookup: impl Fn(u32) -> Option<ProcessInfo>) -> Option<u32> {
    let mut current = Some(pid);
    // Bounded, in case /proc changes during the search
    for _ in 0..64 {
        let pid = current?;
        let info = lookup(pid)?;
        for (app_id, owner) in apps {
            let matches = match owner {
                Owner::Steam(steam_id) => info.steam_app_id == Some(*steam_id),
                Owner::Scope(unit) => info.cgroup.lines().any(|line| line.ends_with(&format!("/{unit}"))),
                Owner::Process(root) => pid == *root,
            };
            if matches {
                return Some(*app_id);
            }
        }
        current = info.parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn table(entries: &[(u32, ProcessInfo)]) -> impl Fn(u32) -> Option<ProcessInfo> + use<> {
        let map: HashMap<u32, ProcessInfo> = entries.iter().cloned().collect();
        move |pid| map.get(&pid).cloned()
    }

    fn process(parent: Option<u32>, steam: Option<u32>, cgroup: &str) -> ProcessInfo {
        ProcessInfo { parent, steam_app_id: steam, cgroup: cgroup.into() }
    }

    #[test]
    fn parses_proc_files() {
        assert_eq!(parent_from_stat("1234 (Web Content) S 987 1234 1234 0"), Some(987));
        assert_eq!(parent_from_stat("1 (systemd) S 0 1 1"), None);
        assert_eq!(steam_app_id(b"A=1\0SteamAppId=730\0B=2\0"), Some(730));
        assert_eq!(steam_app_id(b"SteamAppId=0\0"), None);
    }

    #[test]
    fn finds_steam_game_by_environment() {
        let lookup = table(&[(50, process(Some(40), Some(730), "")), (40, process(None, None, ""))]);
        let apps = [(730, Owner::Steam(730))];
        assert_eq!(owner_of(50, &apps, lookup), Some(730));
    }

    #[test]
    fn finds_scope_app_through_parents() {
        let scope = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/consoleui-app-9.scope";
        let lookup = table(&[(70, process(Some(60), None, scope)), (60, process(Some(1), None, scope))]);
        let apps = [(FIRST_DESKTOP_APP_ID, Owner::Scope("consoleui-app-9.scope".into()))];
        assert_eq!(owner_of(70, &apps, lookup), Some(FIRST_DESKTOP_APP_ID));
    }

    #[test]
    fn finds_direct_child_and_ignores_strangers() {
        let lookup =
            table(&[(81, process(Some(80), None, "")), (80, process(Some(5), None, "")), (5, process(None, None, ""))]);
        let apps = [(42, Owner::Process(80))];
        assert_eq!(owner_of(81, &apps, &lookup), Some(42));
        assert_eq!(owner_of(5, &apps, &lookup), None);
        assert_eq!(owner_of(999, &apps, &lookup), None);
    }
}
