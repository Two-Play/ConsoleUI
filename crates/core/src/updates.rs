//! System updates without knowing the distribution (Story 5.8): the sources are PackageKit (system
//! packages), Flatpak (apps), fwupd (firmware) and rpm-ostree (image-based systems). A source that
//! is not there is left out. The backend runs on its own thread; the real one lives in the shell,
//! [`FakeBackend`] is the simulation (`--fake-updates`). This file also holds the state of the
//! "Updates" page and the parsers of the tools' output.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::i18n::{tr, trf, trn};
use crate::nav::Action;
use crate::notification;

/// First search after the start, then how often the shell looks for updates
const FIRST_CHECK: Duration = Duration::from_secs(120);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// More of a changelog does not fit into the dialog
const MAX_DETAIL_LINES: usize = 12;
const MAX_DETAIL_CHARS: usize = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// Firmware first: it needs the most attention
    Firmware,
    /// The whole system as an image (rpm-ostree)
    Image,
    /// Packages through PackageKit
    System,
    Flatpak,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firmware => tr("Firmware"),
            Self::Image => tr("System image"),
            Self::System => tr("System package"),
            Self::Flatpak => tr("App"),
        }
    }

    /// Takes effect only after a restart, so it must not disturb the running session
    pub fn needs_restart(self) -> bool {
        self != Self::Flatpak
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    /// What the source knows it by: PackageKit package ID, `origin/app` for Flatpak, fwupd device ID
    pub id: String,
    pub source: Source,
    pub name: String,
    pub version: String,
    pub security: bool,
}

/// What changed in an update
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Details {
    pub text: String,
    pub cves: Vec<String>,
}

impl Details {
    /// The text for the dialog, shortened to what fits.
    pub fn summary(&self) -> String {
        let mut text: String = self
            .text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .take(MAX_DETAIL_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        if text.chars().count() > MAX_DETAIL_CHARS {
            text = text.chars().take(MAX_DETAIL_CHARS).collect::<String>() + " …";
        }
        if text.is_empty() {
            text = tr("No description of the changes available.").into();
        }
        if !self.cves.is_empty() {
            text = format!("{text}\n\n{}", self.cves.join(", "));
        }
        text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Check,
    Details(Update),
    Install(Vec<Update>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    pub updates: Vec<Update>,
    /// Sources that are there but failed
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Checked(Checked),
    Details(String, Result<Details, String>),
    /// What the installation is doing right now
    Progress(String),
    /// `Ok(true)`: the rest happens at the next restart
    Installed(Result<bool, String>),
}

/// Runs on the backend thread, so it may block.
pub trait Backend: Send {
    fn check(&mut self) -> Checked;
    fn details(&mut self, update: &Update) -> Result<Details, String>;
    /// Installs Flatpaks at once and prepares the rest for the next restart; returns whether a
    /// restart is needed.
    fn install(&mut self, updates: &[Update], progress: &mut dyn FnMut(String)) -> Result<bool, String>;
}

/// Starts the backend thread.
pub fn spawn(mut backend: Box<dyn Backend>) -> (Sender<Request>, Receiver<Response>) {
    let (request_tx, request_rx) = mpsc::channel::<Request>();
    let (response_tx, response_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("consoleui-updates".into())
        .spawn(move || {
            while let Ok(request) = request_rx.recv() {
                let response = match request {
                    Request::Check => Response::Checked(backend.check()),
                    Request::Details(update) => Response::Details(update.id.clone(), backend.details(&update)),
                    Request::Install(updates) => {
                        let progress_tx = response_tx.clone();
                        let mut progress = |text| {
                            let _ = progress_tx.send(Response::Progress(text));
                        };
                        let result = backend.install(&updates, &mut progress);
                        Response::Installed(result)
                    }
                };
                if response_tx.send(response).is_err() {
                    return;
                }
            }
        })
        .expect("cannot start the update thread");
    (request_tx, response_rx)
}

/// Notice for a toast
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub symbol: &'static str,
    pub title: String,
    pub text: String,
}

/// What the shell does after a response or an input
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Notice(Notice),
    /// Show what changed (title, text)
    ShowDetails(String, String),
}

/// A row of the page
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub title: String,
    pub detail: String,
    pub security: bool,
    pub busy: bool,
}

/// State of the "Updates" page and of the search in the background
#[derive(Default)]
pub struct UpdatePage {
    open: bool,
    updates: Vec<Update>,
    focused: usize,
    checking: bool,
    checked_once: bool,
    /// How many updates the last toast was about, so a new search does not repeat it
    announced: usize,
    next_check: Option<Instant>,
    /// The update whose details are on the way
    loading: Option<String>,
    /// What the running installation does; `None` when none runs
    installing: Option<String>,
    /// Restart to finish what was installed
    restart_pending: bool,
    errors: Vec<String>,
}

impl UpdatePage {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) -> Vec<Request> {
        self.open = true;
        self.focused = 0;
        self.check()
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn updates(&self) -> &[Update] {
        &self.updates
    }

    pub fn installing(&self) -> bool {
        self.installing.is_some()
    }

    fn check(&mut self) -> Vec<Request> {
        if self.checking || self.installing() {
            return Vec::new();
        }
        self.checking = true;
        vec![Request::Check]
    }

    /// The search in the background is due.
    pub fn check_due(&mut self, now: Instant) -> Vec<Request> {
        let due = *self.next_check.get_or_insert(now + FIRST_CHECK);
        if now < due {
            return Vec::new();
        }
        self.next_check = Some(now + CHECK_EVERY);
        self.check()
    }

    /// The summary in the settings row
    pub fn summary(&self) -> String {
        if self.installing() {
            tr("Installing …").into()
        } else if self.restart_pending {
            tr("Restart needed").into()
        } else if !self.updates.is_empty() {
            trn("{n} available", "{n} available", self.updates.len(), &[])
        } else if self.checking {
            tr("Searching …").into()
        } else if self.checked_once && self.errors.is_empty() {
            tr("Up to date").into()
        } else {
            String::new()
        }
    }

    pub fn handle(&mut self, response: Response) -> Option<Outcome> {
        match response {
            Response::Checked(checked) => {
                self.checking = false;
                self.checked_once = true;
                self.updates = checked.updates;
                self.updates.sort_by(|a, b| (a.source, !a.security, &a.name).cmp(&(b.source, !b.security, &b.name)));
                self.errors = checked.errors;
                self.focused = self.focused.min(self.updates.len().saturating_sub(1));
                let count = self.updates.len();
                let news = count > self.announced;
                self.announced = count;
                news.then(|| {
                    Outcome::Notice(Notice {
                        symbol: "↓",
                        title: trn("{n} update available", "{n} updates available", count, &[]),
                        text: tr("Settings → Updates").into(),
                    })
                })
            }
            Response::Details(id, result) => {
                if self.loading.as_ref() != Some(&id) {
                    return None;
                }
                self.loading = None;
                let name = self.updates.iter().find(|u| u.id == id).map(|u| u.name.clone()).unwrap_or_default();
                Some(match result {
                    Ok(details) => Outcome::ShowDetails(name, details.summary()),
                    Err(err) => Outcome::ShowDetails(name, err),
                })
            }
            Response::Progress(text) => {
                self.installing = Some(text);
                None
            }
            Response::Installed(result) => {
                self.installing = None;
                let notice = match result {
                    Ok(restart) => {
                        self.restart_pending |= restart;
                        self.updates.clear();
                        self.announced = 0;
                        let text = if restart { tr("Restart to finish") } else { tr("Everything is up to date") };
                        Notice { symbol: "✓", title: tr("Updates installed").into(), text: text.into() }
                    }
                    Err(err) => Notice { symbol: "⚠", title: tr("Updating failed").into(), text: err },
                };
                Some(Outcome::Notice(notice))
            }
        }
    }

    /// Input on the open page. □ (install) is up to the shell: it asks first and knows whether a
    /// game runs.
    pub fn apply(&mut self, action: Action) -> Vec<Request> {
        match action {
            Action::Up => self.focused = self.focused.saturating_sub(1),
            Action::Down => self.focused = (self.focused + 1).min(self.updates.len().saturating_sub(1)),
            Action::Back => self.close(),
            Action::Menu => return self.check(),
            Action::Confirm => {
                if let Some(update) = self.updates.get(self.focused).filter(|_| self.loading.is_none()) {
                    self.loading = Some(update.id.clone());
                    return vec![Request::Details(update.clone())];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// Starts the installation of everything found.
    pub fn install(&mut self) -> Vec<Request> {
        if self.updates.is_empty() || self.installing() {
            return Vec::new();
        }
        self.installing = Some(tr("Please wait …").into());
        vec![Request::Install(self.updates.clone())]
    }

    /// The line below the summary while the list is empty
    pub fn empty_text(&self) -> &'static str {
        if self.restart_pending { tr("Restart to finish") } else { tr("New versions show up here.") }
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn rows(&self) -> Vec<RowView> {
        self.updates
            .iter()
            .map(|update| RowView {
                title: update.name.clone(),
                detail: match update.version.as_str() {
                    "" => update.source.label().to_owned(),
                    version => format!("{} · {version}", update.source.label()),
                },
                security: update.security,
                busy: self.loading.as_ref() == Some(&update.id),
            })
            .collect()
    }

    /// The line at the top of the page
    pub fn status(&self) -> String {
        if let Some(step) = &self.installing {
            return step.clone();
        }
        if self.checking {
            return tr("Searching …").into();
        }
        if self.updates.iter().any(|update| update.source.needs_restart()) {
            return tr("System updates take effect at the next restart").into();
        }
        match self.errors.first() {
            Some(error) => trf("Not everything could be checked: {}", &[error]),
            // Without updates the page shows the summary as its big line
            None if self.updates.is_empty() => String::new(),
            None => self.summary(),
        }
    }
}

/// `flatpak remote-ls --updates --columns=application,name,version,origin`: one tab-separated line
/// each. The ID is `origin/application`, as `flatpak remote-info` needs both.
pub fn parse_flatpak(output: &str) -> Vec<Update> {
    output
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t').map(str::trim);
            let app = columns.next().filter(|app| !app.is_empty())?;
            let name = columns.next().filter(|name| !name.is_empty()).unwrap_or(app);
            let version = columns.next().unwrap_or_default();
            let origin = columns.next().unwrap_or_default();
            Some(Update {
                id: format!("{origin}/{app}"),
                source: Source::Flatpak,
                name: name.to_owned(),
                version: version.to_owned(),
                security: false,
            })
        })
        .collect()
}

/// `flatpak remote-info`: the commit's subject is all it tells about the changes.
pub fn flatpak_details(output: &str) -> Details {
    let subject = output.lines().find_map(|line| line.trim().strip_prefix("Subject:"));
    Details { text: subject.unwrap_or_default().trim().to_owned(), cves: Vec::new() }
}

/// `fwupdmgr get-updates --json`: the devices with their newest release.
pub fn parse_fwupd(output: &str) -> Vec<Update> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(output) else { return Vec::new() };
    let devices = root["Devices"].as_array().map(Vec::as_slice).unwrap_or_default();
    devices
        .iter()
        .filter_map(|device| {
            let release = device["Releases"].as_array()?.first()?;
            Some(Update {
                id: device["DeviceId"].as_str()?.to_owned(),
                source: Source::Firmware,
                name: device["Name"].as_str().unwrap_or("Firmware").to_owned(),
                version: release["Version"].as_str().unwrap_or_default().to_owned(),
                security: false,
            })
        })
        .collect()
}

/// What the newest release of a device changes, from the same output as [`parse_fwupd`].
pub fn fwupd_details(output: &str, device_id: &str) -> Option<Details> {
    let root = serde_json::from_str::<serde_json::Value>(output).ok()?;
    let device = root["Devices"].as_array()?.iter().find(|device| device["DeviceId"] == device_id)?;
    let release = device["Releases"].as_array()?.first()?;
    let text = release["Description"].as_str().or(release["Summary"].as_str()).unwrap_or_default();
    Some(Details { text: notification::plain_text(text), cves: Vec::new() })
}

/// `rpm-ostree upgrade --check` (run with `LC_ALL=C`): a block with `Version: 41.2 (date)`.
pub fn parse_rpm_ostree(output: &str) -> Option<Update> {
    let available = output.split("AvailableUpdate:").nth(1)?;
    let version = available.lines().find_map(|line| line.trim().strip_prefix("Version:"))?;
    let version = version.split_whitespace().next().unwrap_or_default();
    Some(Update {
        id: "rpm-ostree".into(),
        source: Source::Image,
        name: tr("Operating system").into(),
        version: version.to_owned(),
        security: false,
    })
}

/// PackageKit's package ID `name;version;arch;repository` as name and version.
pub fn split_package_id(id: &str) -> (&str, &str) {
    let mut parts = id.split(';');
    (parts.next().unwrap_or(id), parts.next().unwrap_or_default())
}

/// Packages of several architectures have the same name: those get the architecture added.
pub fn name_architectures(updates: &mut [Update]) {
    let names: Vec<String> = updates.iter().map(|update| update.name.clone()).collect();
    for update in updates {
        let arch = update.id.split(';').nth(2).unwrap_or_default();
        if names.iter().filter(|&name| *name == update.name).count() > 1 && !arch.is_empty() {
            update.name = format!("{} ({arch})", update.name);
        }
    }
}

/// The simulated backend: three updates that are gone after installing.
pub struct FakeBackend {
    delay: Duration,
    installed: bool,
}

impl FakeBackend {
    pub fn new(delay: Duration) -> Self {
        Self { delay, installed: false }
    }
}

impl Backend for FakeBackend {
    fn check(&mut self) -> Checked {
        std::thread::sleep(self.delay);
        if self.installed {
            return Checked::default();
        }
        let update = |id: &str, source, name: &str, version: &str, security| Update {
            id: id.into(),
            source,
            name: name.into(),
            version: version.into(),
            security,
        };
        Checked {
            updates: vec![
                update("kernel;6.20.1;x86_64;updates", Source::System, "kernel", "6.20.1", true),
                update("org.example.Player", Source::Flatpak, "Example Player", "2.4", false),
                update("controller", Source::Firmware, "Example Controller", "1.8", false),
            ],
            errors: Vec::new(),
        }
    }

    fn details(&mut self, update: &Update) -> Result<Details, String> {
        std::thread::sleep(self.delay);
        Ok(Details {
            text: format!("Changes in {}\n- Fixes\n- Faster", update.name),
            cves: vec!["CVE-2026-0001".into()],
        })
    }

    fn install(&mut self, updates: &[Update], progress: &mut dyn FnMut(String)) -> Result<bool, String> {
        for update in updates {
            progress(trf("Installing {}", &[&update.name]));
            std::thread::sleep(self.delay);
        }
        self.installed = true;
        Ok(updates.iter().any(|update| update.source.needs_restart()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(backend: &mut FakeBackend) -> UpdatePage {
        let mut page = UpdatePage::default();
        assert_eq!(page.open(), vec![Request::Check]);
        page.handle(Response::Checked(backend.check()));
        page
    }

    #[test]
    fn parses_flatpak_lines() {
        let updates = parse_flatpak("org.a.App\tApp\t1.2\tflathub\n\norg.b.NoName\t\t\t\n");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].id, "flathub/org.a.App");
        assert_eq!(flatpak_details("  Subject: Update to 1.2 (abc)\n Date: x\n").text, "Update to 1.2 (abc)");
        assert_eq!((updates[0].name.as_str(), updates[0].version.as_str()), ("App", "1.2"));
        assert_eq!(updates[1].name, "org.b.NoName");
    }

    #[test]
    fn parses_fwupd_json() {
        let json = r#"{"Devices":[{"Name":"Pad","DeviceId":"abc","Releases":[{"Version":"1.2",
            "Description":"<p>Better <b>rumble</b></p>"}]},{"Name":"No release","DeviceId":"x"}]}"#;
        let updates = parse_fwupd(json);
        assert_eq!(updates.len(), 1);
        assert_eq!((updates[0].id.as_str(), updates[0].version.as_str()), ("abc", "1.2"));
        assert!(fwupd_details(json, "abc").unwrap().text.contains("Better rumble"));
        assert_eq!(fwupd_details(json, "nope"), None);
        assert!(parse_fwupd("{\"Devices\":[]}").is_empty());
        assert!(parse_fwupd("no json").is_empty());
    }

    #[test]
    fn parses_rpm_ostree_and_package_ids() {
        let output = "Note\nAvailableUpdate:\n        Version: 41.20261101 (2026-11-01)\n        Commit: abc\n";
        assert_eq!(parse_rpm_ostree(output).unwrap().version, "41.20261101");
        assert_eq!(parse_rpm_ostree("No updates available.\n"), None);
        assert_eq!(split_package_id("kernel;6.1;x86_64;updates"), ("kernel", "6.1"));
    }

    #[test]
    fn tells_architectures_of_the_same_package_apart() {
        let package = |id: &str, name: &str| Update {
            id: id.into(),
            source: Source::System,
            name: name.into(),
            version: String::new(),
            security: false,
        };
        let mut updates = [package("a;1;x86_64;u", "a"), package("a;1;i686;u", "a"), package("b;1;x86_64;u", "b")];
        name_architectures(&mut updates);
        let names: Vec<&str> = updates.iter().map(|update| update.name.as_str()).collect();
        assert_eq!(names, ["a (x86_64)", "a (i686)", "b"]);
    }

    #[test]
    fn details_are_shortened_and_list_cves() {
        let long = Details { text: "line\n".repeat(50), cves: vec!["CVE-1".into()] };
        assert_eq!(long.summary().lines().count(), MAX_DETAIL_LINES + 2);
        assert!(long.summary().ends_with("CVE-1"));
        assert!(Details::default().summary().starts_with("No description"));
    }

    #[test]
    fn announces_only_new_updates() {
        let mut backend = FakeBackend::new(Duration::ZERO);
        let mut page = UpdatePage::default();
        let first = page.handle(Response::Checked(backend.check()));
        assert!(matches!(first, Some(Outcome::Notice(notice)) if notice.title == "3 updates available"));
        assert_eq!(page.handle(Response::Checked(backend.check())), None);
        assert_eq!(page.summary(), "3 available");
    }

    #[test]
    fn sorts_and_shows_details_of_the_focused_row() {
        let mut backend = FakeBackend::new(Duration::ZERO);
        let mut page = checked(&mut backend);
        assert_eq!(page.updates()[0].source, Source::Firmware);
        page.apply(Action::Down);
        let requests = page.apply(Action::Confirm);
        let [Request::Details(update)] = requests.as_slice() else { panic!("{requests:?}") };
        assert_eq!(update.name, "kernel");
        assert!(page.rows()[1].busy);
        assert!(page.apply(Action::Confirm).is_empty());
        let details = backend.details(update);
        let outcome = page.handle(Response::Details(update.id.clone(), details));
        assert!(matches!(outcome, Some(Outcome::ShowDetails(name, text)) if name == "kernel" && text.contains("CVE")));
        assert!(!page.rows()[1].busy);
    }

    #[test]
    fn install_clears_the_list_and_asks_for_a_restart() {
        let mut backend = FakeBackend::new(Duration::ZERO);
        let mut page = checked(&mut backend);
        let requests = page.install();
        let [Request::Install(updates)] = requests.as_slice() else { panic!("{requests:?}") };
        assert!(page.install().is_empty());
        assert!(page.apply(Action::Menu).is_empty());
        page.handle(Response::Progress("Installing kernel".into()));
        assert_eq!(page.status(), "Installing kernel");
        let result = backend.install(updates, &mut |_| {});
        page.handle(Response::Installed(result));
        assert!(page.updates().is_empty());
        assert_eq!(page.summary(), "Restart needed");
    }

    #[test]
    fn checks_in_the_background_only_when_due() {
        let mut page = UpdatePage::default();
        let start = Instant::now();
        assert!(page.check_due(start).is_empty());
        assert_eq!(page.check_due(start + FIRST_CHECK), vec![Request::Check]);
        assert!(page.check_due(start + FIRST_CHECK + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn worker_answers() {
        let (tx, rx) = spawn(Box::new(FakeBackend::new(Duration::ZERO)));
        tx.send(Request::Check).unwrap();
        let response = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(response, Response::Checked(checked) if checked.updates.len() == 3));
    }
}
