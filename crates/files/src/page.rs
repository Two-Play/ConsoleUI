//! What the file manager shows and how it reacts to input, without any UI.

use std::path::{Path, PathBuf};

use consoleui_core::dialog::{Dialog, DialogButton, DialogOutcome};
use consoleui_core::i18n::{tr, trf};
use consoleui_core::osk::{KeyboardEvent, OnScreenKeyboard};
use consoleui_core::{Action, sound::Sound};

use crate::entries::{Entry, Kind};
use crate::ops::Place;
use crate::worker::{Request, Response};

/// Rows on the screen; only these reach the UI, a folder can hold thousands of files.
pub const ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Places,
    Files,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Changed(Sound),
    Send(Request),
    Open(PathBuf),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Open,
    Rename,
    Copy,
    Cut,
    Paste,
    NewFolder,
    Trash,
    Hidden,
}

impl MenuItem {
    pub fn label(self, hidden: bool) -> &'static str {
        match self {
            Self::Open => tr("Open"),
            Self::Rename => tr("Rename"),
            Self::Copy => tr("Copy"),
            Self::Cut => tr("Cut"),
            Self::Paste => tr("Paste"),
            Self::NewFolder => tr("New folder"),
            Self::Trash => tr("Move to trash"),
            Self::Hidden if hidden => tr("Hide hidden files"),
            Self::Hidden => tr("Show hidden files"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Cancel,
    Trash(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Naming {
    Rename(PathBuf),
    NewFolder,
}

pub struct Page {
    places: Vec<Place>,
    place: usize,
    pub area: Area,
    dir: PathBuf,
    entries: Vec<Entry>,
    focused: usize,
    top: usize,
    pub hidden: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// A file to paste; `true` moves it
    clipboard: Option<(PathBuf, bool)>,
    menu: Option<(Vec<MenuItem>, usize)>,
    pub dialog: Option<Dialog<Confirm>>,
    naming: Option<Naming>,
    pub text: String,
    pub keyboard: OnScreenKeyboard,
    /// After the next list: the focus goes to this path
    focus_next: Option<PathBuf>,
    /// A change is running
    pub busy: bool,
}

impl Page {
    pub fn new(places: Vec<Place>) -> Self {
        let dir = places.first().map(|place| place.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        Self {
            places,
            place: 0,
            area: Area::Files,
            dir,
            entries: Vec::new(),
            focused: 0,
            top: 0,
            hidden: false,
            loading: true,
            error: None,
            clipboard: None,
            menu: None,
            dialog: None,
            naming: None,
            text: String::new(),
            keyboard: OnScreenKeyboard::default(),
            focus_next: None,
            busy: false,
        }
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    pub fn place(&self) -> usize {
        self.place
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn total(&self) -> usize {
        self.entries.len()
    }

    pub fn visible(&self) -> &[Entry] {
        let end = (self.top + ROWS).min(self.entries.len());
        &self.entries[self.top.min(end)..end]
    }

    pub fn focused(&self) -> usize {
        self.focused
    }

    pub fn visible_focus(&self) -> usize {
        self.focused.saturating_sub(self.top)
    }

    pub fn menu(&self) -> Option<(&[MenuItem], usize)> {
        self.menu.as_ref().map(|(items, focused)| (items.as_slice(), *focused))
    }

    pub fn typing(&self) -> bool {
        self.naming.is_some()
    }

    pub fn naming_title(&self) -> &'static str {
        match self.naming {
            Some(Naming::NewFolder) => tr("New folder"),
            _ => tr("Rename"),
        }
    }

    fn entry(&self) -> Option<&Entry> {
        self.entries.get(self.focused)
    }

    /// The request for the current folder.
    pub fn list(&self) -> Request {
        Request::List { dir: self.dir.clone(), hidden: self.hidden }
    }

    fn go(&mut self, dir: PathBuf, focus: Option<PathBuf>) -> Outcome {
        self.dir = dir;
        self.entries.clear();
        self.focused = 0;
        self.top = 0;
        self.loading = true;
        self.error = None;
        self.focus_next = focus;
        Outcome::Send(self.list())
    }

    fn follow_focus(&mut self) {
        if self.focused < self.top {
            self.top = self.focused;
        } else if self.focused >= self.top + ROWS {
            self.top = self.focused + 1 - ROWS;
        }
    }

    /// Takes over what the worker reports; a finished change asks for a new look at the folder.
    pub fn receive(&mut self, response: Response) -> Option<Request> {
        match response {
            Response::Listed(dir, _) if dir != self.dir => None,
            Response::Listed(_, Err(err)) => {
                self.loading = false;
                self.entries.clear();
                self.error = Some(err);
                None
            }
            Response::Listed(_, Ok(entries)) => {
                let keep = self.focus_next.take().or_else(|| self.entry().map(|entry| entry.path.clone()));
                self.entries = entries;
                self.loading = false;
                let position = keep.and_then(|path| self.entries.iter().position(|entry| entry.path == path));
                self.focused = position.unwrap_or(self.focused).min(self.entries.len().saturating_sub(1));
                self.follow_focus();
                None
            }
            Response::Done(result) => {
                self.busy = false;
                if let Err(err) = result {
                    self.dialog = Some(Dialog::alert(tr("That did not work"), err, tr("OK"), Confirm::Cancel));
                }
                Some(self.list())
            }
        }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        if self.dialog.is_some() {
            return self.apply_dialog(action);
        }
        if self.naming.is_some() {
            return self.apply_keyboard(action);
        }
        if self.menu.is_some() {
            return self.apply_menu(action);
        }
        match self.area {
            Area::Places => self.apply_places(action),
            Area::Files => self.apply_files(action),
        }
    }

    fn apply_dialog(&mut self, action: Action) -> Outcome {
        let Some(dialog) = self.dialog.as_mut() else { return Outcome::Nothing };
        match dialog.apply(action) {
            DialogOutcome::None => Outcome::Changed(Sound::Move),
            DialogOutcome::Dismissed | DialogOutcome::Chosen(Confirm::Cancel) => {
                self.dialog = None;
                Outcome::Changed(Sound::Back)
            }
            DialogOutcome::Chosen(Confirm::Trash(path)) => {
                self.dialog = None;
                self.send_change(Request::Trash(path))
            }
        }
    }

    fn send_change(&mut self, request: Request) -> Outcome {
        self.busy = true;
        Outcome::Send(request)
    }

    fn apply_keyboard(&mut self, action: Action) -> Outcome {
        if action == Action::Back {
            self.naming = None;
            return Outcome::Changed(Sound::Back);
        }
        match self.keyboard.apply(action) {
            KeyboardEvent::None => Outcome::Nothing,
            KeyboardEvent::Moved | KeyboardEvent::Shift => Outcome::Changed(Sound::Move),
            KeyboardEvent::Type(c) => {
                self.text.push(c);
                Outcome::Changed(Sound::Confirm)
            }
            KeyboardEvent::Space => {
                self.text.push(' ');
                Outcome::Changed(Sound::Confirm)
            }
            KeyboardEvent::Backspace => {
                self.text.pop();
                Outcome::Changed(Sound::Confirm)
            }
            KeyboardEvent::Done => self.finish_naming(),
        }
    }

    /// Enter on a real keyboard ends up here as well.
    pub fn finish_naming(&mut self) -> Outcome {
        let name = self.text.trim().to_string();
        let Some(naming) = self.naming.take() else { return Outcome::Nothing };
        if name.is_empty() {
            return Outcome::Changed(Sound::Back);
        }
        self.focus_next = Some(match &naming {
            Naming::Rename(path) => path.with_file_name(&name),
            Naming::NewFolder => self.dir.join(&name),
        });
        match naming {
            Naming::Rename(path) => self.send_change(Request::Rename { path, name }),
            Naming::NewFolder => self.send_change(Request::NewFolder { dir: self.dir.clone(), name }),
        }
    }

    /// Text from a real keyboard.
    pub fn typed(&mut self, text: &str) {
        self.text.push_str(text);
    }

    pub fn erase(&mut self) {
        self.text.pop();
    }

    fn start_naming(&mut self, naming: Naming, text: String) -> Outcome {
        self.naming = Some(naming);
        self.text = text;
        self.keyboard.reset_focus();
        Outcome::Changed(Sound::Open)
    }

    fn open_menu(&mut self) -> Outcome {
        let mut items = Vec::new();
        if self.entry().is_some() {
            items.extend([MenuItem::Open, MenuItem::Rename, MenuItem::Copy, MenuItem::Cut]);
        }
        if self.clipboard.is_some() {
            items.push(MenuItem::Paste);
        }
        items.push(MenuItem::NewFolder);
        if self.entry().is_some() {
            items.push(MenuItem::Trash);
        }
        items.push(MenuItem::Hidden);
        self.menu = Some((items, 0));
        Outcome::Changed(Sound::Open)
    }

    fn apply_menu(&mut self, action: Action) -> Outcome {
        let Some((items, focused)) = self.menu.as_mut() else { return Outcome::Nothing };
        match action {
            Action::Up if *focused > 0 => *focused -= 1,
            Action::Down if *focused + 1 < items.len() => *focused += 1,
            Action::Back | Action::Menu => {
                self.menu = None;
                return Outcome::Changed(Sound::Back);
            }
            Action::Confirm => {
                let item = items[*focused];
                self.menu = None;
                return self.run(item);
            }
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Move)
    }

    /// A click on a menu row.
    pub fn click_menu(&mut self, index: usize) -> Outcome {
        let item = self.menu.take().and_then(|(items, _)| items.get(index).copied());
        item.map_or(Outcome::Nothing, |item| self.run(item))
    }

    fn run(&mut self, item: MenuItem) -> Outcome {
        let entry = self.entry().cloned();
        match (item, entry) {
            (MenuItem::Open, Some(entry)) => self.open(entry),
            (MenuItem::Rename, Some(entry)) => self.start_naming(Naming::Rename(entry.path), entry.name),
            (MenuItem::Copy, Some(entry)) => {
                self.clipboard = Some((entry.path, false));
                Outcome::Changed(Sound::Confirm)
            }
            (MenuItem::Cut, Some(entry)) => {
                self.clipboard = Some((entry.path, true));
                Outcome::Changed(Sound::Confirm)
            }
            (MenuItem::Trash, Some(entry)) => {
                let buttons = vec![
                    DialogButton { label: tr("Cancel").to_string(), tag: Confirm::Cancel },
                    DialogButton { label: tr("Move to trash").to_string(), tag: Confirm::Trash(entry.path) },
                ];
                let title = trf("Move {} to the trash?", &[&entry.name]);
                self.dialog = Some(Dialog::new(title, tr("You can restore it from the trash."), buttons, 0));
                Outcome::Changed(Sound::Open)
            }
            (MenuItem::Paste, _) => self.paste(),
            (MenuItem::NewFolder, _) => self.start_naming(Naming::NewFolder, String::new()),
            (MenuItem::Hidden, _) => {
                self.hidden = !self.hidden;
                Outcome::Send(self.list())
            }
            _ => Outcome::Nothing,
        }
    }

    fn paste(&mut self) -> Outcome {
        let Some((from, moving)) = self.clipboard.clone() else { return Outcome::Nothing };
        let into = self.dir.clone();
        if moving {
            // The file is gone from where it was: it can only be moved once
            self.clipboard = None;
            self.send_change(Request::Move { from, into })
        } else {
            self.send_change(Request::Copy { from, into })
        }
    }

    fn open(&mut self, entry: Entry) -> Outcome {
        if entry.kind == Kind::Folder { self.go(entry.path, None) } else { Outcome::Open(entry.path) }
    }

    fn apply_places(&mut self, action: Action) -> Outcome {
        match action {
            Action::Up if self.place > 0 => self.place -= 1,
            Action::Down if self.place + 1 < self.places.len() => self.place += 1,
            Action::Right => self.area = Area::Files,
            Action::Confirm => return self.open_place(self.place),
            Action::Back => return Outcome::Quit,
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Move)
    }

    /// A click on the sidebar comes here as well.
    pub fn open_place(&mut self, index: usize) -> Outcome {
        let Some(place) = self.places.get(index) else { return Outcome::Nothing };
        let path = place.path.clone();
        self.place = index;
        self.area = Area::Files;
        self.go(path, None)
    }

    fn apply_files(&mut self, action: Action) -> Outcome {
        let last = self.entries.len().saturating_sub(1);
        match action {
            Action::Up if self.focused > 0 => self.focused -= 1,
            Action::Down if self.focused < last => self.focused += 1,
            Action::Left => self.area = Area::Places,
            Action::Confirm => return self.entry().cloned().map_or(Outcome::Nothing, |entry| self.open(entry)),
            Action::Menu => return self.open_menu(),
            Action::Back => return self.back(),
            _ => return Outcome::Nothing,
        }
        self.follow_focus();
        Outcome::Changed(Sound::Move)
    }

    /// Up one folder; at the top of the place the sidebar takes over.
    fn back(&mut self) -> Outcome {
        let root = self.places.get(self.place).map(|place| place.path.as_path());
        match self.dir.parent().filter(|_| root != Some(self.dir.as_path())) {
            Some(parent) => {
                let (parent, here) = (parent.to_path_buf(), self.dir.clone());
                self.go(parent, Some(here))
            }
            None => {
                self.area = Area::Places;
                Outcome::Changed(Sound::Back)
            }
        }
    }

    /// A click on a row: the first click selects, the one on the selected row opens.
    pub fn click_row(&mut self, index: usize) -> Outcome {
        self.area = Area::Files;
        let target = self.top + index;
        if target == self.focused {
            return self.entry().cloned().map_or(Outcome::Nothing, |entry| self.open(entry));
        }
        self.focused = target.min(self.entries.len().saturating_sub(1));
        Outcome::Changed(Sound::Move)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(dir: &str, name: &str, kind: Kind) -> Entry {
        let path = Path::new(dir).join(name);
        Entry { name: name.into(), path, kind, size: 1, modified: None }
    }

    fn page(count: usize) -> Page {
        let place = |name: &str, path: &str| Place { name: name.into(), path: path.into() };
        let mut page = Page::new(vec![place("Home", "/home/me"), place("Music", "/home/me/Music")]);
        let mut entries = vec![entry("/home/me", "folder", Kind::Folder)];
        entries.extend((1..count).map(|i| entry("/home/me", &format!("file{i}.txt"), Kind::Text)));
        page.receive(Response::Listed("/home/me".into(), Ok(entries)));
        page
    }

    #[test]
    fn the_list_scrolls_and_only_shows_the_screen() {
        let mut page = page(30);
        assert_eq!(page.visible().len(), ROWS);
        for _ in 0..12 {
            page.apply(Action::Down);
        }
        assert_eq!(page.visible_focus(), ROWS - 1);
        assert_eq!(page.visible()[0].name, "file3.txt");
    }

    #[test]
    fn confirm_enters_folders_and_opens_files() {
        let mut page = page(3);
        let list = Request::List { dir: "/home/me/folder".into(), hidden: false };
        assert_eq!(page.apply(Action::Confirm), Outcome::Send(list));
        page.receive(Response::Listed(
            "/home/me/folder".into(),
            Ok(vec![entry("/home/me/folder", "a.txt", Kind::Text)]),
        ));
        assert_eq!(page.apply(Action::Confirm), Outcome::Open("/home/me/folder/a.txt".into()));
    }

    #[test]
    fn back_returns_to_the_folder_it_came_from() {
        let mut page = page(3);
        page.apply(Action::Confirm);
        page.receive(Response::Listed("/home/me/folder".into(), Ok(Vec::new())));
        let list = Request::List { dir: "/home/me".into(), hidden: false };
        assert_eq!(page.apply(Action::Back), Outcome::Send(list));
        let entries = vec![entry("/home/me", "a", Kind::Folder), entry("/home/me", "folder", Kind::Folder)];
        page.receive(Response::Listed("/home/me".into(), Ok(entries)));
        assert_eq!(page.focused(), 1);
    }

    #[test]
    fn back_at_the_top_goes_to_the_sidebar_and_then_quits() {
        let mut page = page(1);
        page.apply(Action::Back);
        assert_eq!(page.area, Area::Places);
        assert_eq!(page.apply(Action::Back), Outcome::Quit);
    }

    #[test]
    fn an_old_list_does_not_replace_the_new_folder() {
        let mut page = page(2);
        page.open_place(1);
        page.receive(Response::Listed("/home/me".into(), Ok(vec![entry("/home/me", "old", Kind::Text)])));
        assert_eq!(page.total(), 0);
        assert!(page.loading);
    }

    fn choose(page: &mut Page, item: MenuItem) -> Outcome {
        page.apply(Action::Menu);
        let index = page.menu().and_then(|(items, _)| items.iter().position(|i| *i == item)).expect("in the menu");
        page.click_menu(index)
    }

    #[test]
    fn cut_pastes_once_and_copy_many_times() {
        let mut page = page(2);
        page.apply(Action::Down);
        choose(&mut page, MenuItem::Cut);
        let from = PathBuf::from("/home/me/file1.txt");
        let moved = Request::Move { from: from.clone(), into: "/home/me".into() };
        assert_eq!(choose(&mut page, MenuItem::Paste), Outcome::Send(moved));
        page.apply(Action::Menu);
        assert!(!page.menu().unwrap().0.contains(&MenuItem::Paste));
        page.apply(Action::Back);
        choose(&mut page, MenuItem::Copy);
        choose(&mut page, MenuItem::Paste);
        let copied = Request::Copy { from, into: "/home/me".into() };
        assert_eq!(choose(&mut page, MenuItem::Paste), Outcome::Send(copied));
    }

    #[test]
    fn the_trash_asks_first() {
        let mut page = page(2);
        page.apply(Action::Down);
        choose(&mut page, MenuItem::Trash);
        assert_eq!(page.apply(Action::Confirm), Outcome::Changed(Sound::Back), "Cancel has the focus");
        choose(&mut page, MenuItem::Trash);
        page.apply(Action::Right);
        let trash = Request::Trash("/home/me/file1.txt".into());
        assert_eq!(page.apply(Action::Confirm), Outcome::Send(trash));
        assert!(page.busy);
    }

    #[test]
    fn renaming_starts_with_the_old_name_and_focuses_the_new_one() {
        let mut page = page(2);
        page.apply(Action::Down);
        choose(&mut page, MenuItem::Rename);
        assert_eq!(page.text, "file1.txt");
        page.erase();
        page.typed("md");
        let rename = Request::Rename { path: "/home/me/file1.txt".into(), name: "file1.txmd".into() };
        assert_eq!(page.finish_naming(), Outcome::Send(rename));
        let list = page.receive(Response::Done(Ok(())));
        assert_eq!(list, Some(page.list()));
        let entries = vec![entry("/home/me", "a", Kind::Text), entry("/home/me", "file1.txmd", Kind::Text)];
        page.receive(Response::Listed("/home/me".into(), Ok(entries)));
        assert_eq!(page.focused(), 1);
    }

    #[test]
    fn an_empty_name_changes_nothing() {
        let mut page = page(1);
        choose(&mut page, MenuItem::NewFolder);
        assert_eq!(page.finish_naming(), Outcome::Changed(Sound::Back));
    }

    #[test]
    fn a_failed_change_is_shown_and_the_list_still_refreshes() {
        let mut page = page(1);
        assert!(page.receive(Response::Done(Err("No".into()))).is_some());
        assert_eq!(page.dialog.as_ref().map(|d| d.text.as_str()), Some("No"));
    }
}
