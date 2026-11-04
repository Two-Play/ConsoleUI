//! Real update backend (Story 5.8): PackageKit over the system bus, `flatpak`, `fwupdmgr` and
//! `rpm-ostree` as programs. Runs only on the update thread (`consoleui_core::updates`). A source
//! that is not installed is left out; the parsing lives in the core.

use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use consoleui_core::i18n::trf;
use consoleui_core::updates::{self, Backend, Checked, Details, Source, Update};
use serde::Serialize;
use zbus::blocking::{Connection, Proxy};
use zbus::message::Message;
use zbus::zvariant::{DynamicType, OwnedObjectPath};

const PK: &str = "org.freedesktop.PackageKit";
const PK_PATH: &str = "/org/freedesktop/PackageKit";
const PK_OFFLINE: &str = "org.freedesktop.PackageKit.Offline";
const PK_TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
/// `PK_INFO_ENUM_SECURITY` and `PK_INFO_ENUM_DOWNLOADING`
const PK_INFO_SECURITY: u32 = 8;
const PK_INFO_DOWNLOADING: u32 = 10;
/// `PK_TRANSACTION_FLAG_ENUM_ONLY_TRUSTED | ONLY_DOWNLOAD`: packages are installed by the offline
/// update at the next restart, not under the running session.
const PK_ONLY_TRUSTED_DOWNLOAD: u64 = (1 << 1) | (1 << 3);
/// Exists on image-based systems, where the packages are not PackageKit's to change
const OSTREE_BOOTED: &str = "/run/ostree-booted";

#[derive(Default)]
pub struct SystemBackend {
    bus: Option<Connection>,
}

impl SystemBackend {
    fn bus(&mut self) -> Result<&Connection, String> {
        if self.bus.is_none() {
            self.bus = Some(Connection::system().map_err(|err| err.to_string())?);
        }
        Ok(self.bus.as_ref().expect("connected above"))
    }

    fn packagekit(&mut self) -> Option<&Connection> {
        if Path::new(OSTREE_BOOTED).exists() {
            return None;
        }
        self.bus().ok().filter(|bus| service_exists(bus, PK))
    }
}

/// Whether the D-Bus name is there or can be started.
fn service_exists(bus: &Connection, name: &str) -> bool {
    let reply = bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListActivatableNames",
        &(),
    );
    let activatable = reply.ok().and_then(|reply| reply.body().deserialize::<Vec<String>>().ok()).unwrap_or_default();
    activatable.iter().any(|known| known == name)
}

/// Runs a PackageKit transaction and hands every signal to `on_signal` until it is finished.
fn transaction<B>(
    bus: &Connection,
    method: &str,
    args: &B,
    mut on_signal: impl FnMut(&str, &Message),
) -> Result<(), String>
where
    B: Serialize + DynamicType,
{
    let text = |err: zbus::Error| err.to_string();
    let reply = bus.call_method(Some(PK), PK_PATH, Some(PK), "CreateTransaction", &()).map_err(text)?;
    let path: OwnedObjectPath = reply.body().deserialize().map_err(text)?;
    let proxy = Proxy::new(bus, PK, path, PK_TRANSACTION).map_err(text)?;
    let hints = vec!["interactive=false".to_owned()];
    proxy.call_method("SetHints", &(hints,)).map_err(text)?;
    let signals = proxy.receive_all_signals().map_err(text)?;
    proxy.call_method(method, args).map_err(text)?;
    let mut error = None;
    for signal in signals {
        let header = signal.header();
        let Some(member) = header.member().map(|member| member.as_str().to_owned()) else { continue };
        match member.as_str() {
            "Finished" => break,
            "ErrorCode" => error = signal.body().deserialize::<(u32, String)>().ok().map(|(_, text)| text),
            _ => on_signal(&member, &signal),
        }
    }
    error.map_or(Ok(()), Err)
}

fn packages(bus: &Connection) -> Result<Vec<Update>, String> {
    // Old metadata only gives old answers; without a network the cache still works.
    if let Err(err) = transaction(bus, "RefreshCache", &(false,), |_, _| {}) {
        eprintln!("PackageKit cannot refresh: {err}");
    }
    let mut updates = Vec::new();
    transaction(bus, "GetUpdates", &(0u64,), |member, signal| {
        if member == "Package"
            && let Ok((info, id, _)) = signal.body().deserialize::<(u32, String, String)>()
        {
            let (name, version) = updates::split_package_id(&id);
            let (name, version) = (name.to_owned(), version.to_owned());
            updates.push(Update { id, source: Source::System, name, version, security: info == PK_INFO_SECURITY });
        }
    })?;
    updates::name_architectures(&mut updates);
    Ok(updates)
}

type UpdateDetail =
    (String, Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>, u32, String, String, u32, String, String);

fn package_details(bus: &Connection, id: &str) -> Result<Details, String> {
    let mut details = Details::default();
    transaction(bus, "GetUpdateDetail", &(vec![id.to_owned()],), |member, signal| {
        if member == "UpdateDetail"
            && let Ok(detail) = signal.body().deserialize::<UpdateDetail>()
        {
            let (update_text, changelog) = (detail.7, detail.8);
            details.text = if update_text.trim().is_empty() { changelog } else { update_text };
            details.cves = detail.5;
        }
    })?;
    Ok(details)
}

fn install_packages(bus: &Connection, updates: &[&Update], progress: &mut dyn FnMut(String)) -> Result<(), String> {
    let ids: Vec<String> = updates.iter().map(|update| update.id.clone()).collect();
    transaction(bus, "UpdatePackages", &(PK_ONLY_TRUSTED_DOWNLOAD, ids), |member, signal| {
        if member == "Package"
            && let Ok((PK_INFO_DOWNLOADING, id, _)) = signal.body().deserialize::<(u32, String, String)>()
        {
            progress(trf("Downloading {}", &[&updates::split_package_id(&id).0]));
        }
    })?;
    let trigger = bus.call_method(Some(PK), PK_PATH, Some(PK_OFFLINE), "Trigger", &("reboot",));
    trigger.map(|_| ()).map_err(|err| err.to_string())
}

/// Runs a tool with English output and no input; `Ok(None)` when it is not installed.
fn tool(program: &str, args: &[&str]) -> Result<Option<Output>, String> {
    let output = Command::new(program).args(args).env("LC_ALL", "C").stdin(Stdio::null()).output();
    match output {
        Ok(output) => Ok(Some(output)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("{program}: {err}")),
    }
}

/// Like [`tool`], for steps that must succeed.
fn must_run(program: &str, args: &[&str]) -> Result<(), String> {
    match tool(program, args)? {
        Some(output) if !output.status.success() => {
            let message = String::from_utf8_lossy(&output.stderr);
            Err(format!("{program}: {}", message.lines().last().unwrap_or("failed")))
        }
        _ => Ok(()),
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn fwupd_output() -> Result<Option<String>, String> {
    // Exit code 2 only means "nothing to do"
    Ok(tool("fwupdmgr", &["get-updates", "--json"])?.map(|output| stdout(&output)))
}

impl Backend for SystemBackend {
    fn check(&mut self) -> Checked {
        let mut checked = Checked::default();
        let mut add = |result: Result<Vec<Update>, String>| match result {
            Ok(updates) => checked.updates.extend(updates),
            Err(err) => checked.errors.push(err),
        };
        if let Some(bus) = self.packagekit() {
            add(packages(bus).map_err(|err| format!("PackageKit: {err}")));
        }
        let flatpak = tool("flatpak", &["remote-ls", "--updates", "--columns=application,name,version,origin"]);
        add(flatpak.map(|output| output.map(|o| updates::parse_flatpak(&stdout(&o))).unwrap_or_default()));
        add(fwupd_output().map(|output| output.map(|o| updates::parse_fwupd(&o)).unwrap_or_default()));
        if Path::new(OSTREE_BOOTED).exists() {
            let image = tool("rpm-ostree", &["upgrade", "--check"]);
            add(image.map(|output| output.and_then(|o| updates::parse_rpm_ostree(&stdout(&o))).into_iter().collect()));
        }
        checked
    }

    fn details(&mut self, update: &Update) -> Result<Details, String> {
        match update.source {
            Source::System => package_details(self.bus()?, &update.id),
            Source::Firmware => {
                let output = fwupd_output()?.unwrap_or_default();
                Ok(updates::fwupd_details(&output, &update.id).unwrap_or_default())
            }
            Source::Flatpak => {
                let Some((origin, app)) = update.id.split_once('/') else { return Ok(Details::default()) };
                let output = tool("flatpak", &["remote-info", origin, app])?;
                Ok(output.map(|o| updates::flatpak_details(&stdout(&o))).unwrap_or_default())
            }
            Source::Image => {
                let output = tool("rpm-ostree", &["upgrade", "--check", "--preview"])?;
                Ok(Details { text: output.map(|o| stdout(&o)).unwrap_or_default(), cves: Vec::new() })
            }
        }
    }

    fn install(&mut self, updates: &[Update], progress: &mut dyn FnMut(String)) -> Result<bool, String> {
        let of = |source| updates.iter().filter(move |update: &&Update| update.source == source);
        if of(Source::Flatpak).next().is_some() {
            progress(trf("Installing {}", &[&Source::Flatpak.label()]));
            must_run("flatpak", &["update", "--noninteractive", "--assumeyes"])?;
        }
        let packages: Vec<&Update> = of(Source::System).collect();
        if !packages.is_empty() {
            install_packages(self.bus()?, &packages, progress)?;
        }
        if of(Source::Image).next().is_some() {
            progress(trf("Installing {}", &[&Source::Image.label()]));
            must_run("rpm-ostree", &["upgrade"])?;
        }
        if of(Source::Firmware).next().is_some() {
            progress(trf("Installing {}", &[&Source::Firmware.label()]));
            must_run("fwupdmgr", &["update", "--assume-yes", "--no-reboot-check"])?;
        }
        Ok(updates.iter().any(|update| update.source.needs_restart()))
    }
}
