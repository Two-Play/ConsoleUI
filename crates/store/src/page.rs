//! What the store shows and how it reacts to input, without any UI.

use std::collections::HashMap;

use consoleui_core::dialog::{Dialog, DialogButton, DialogOutcome};
use consoleui_core::i18n::{tr, trf};
use consoleui_core::osk::{KeyboardEvent, OnScreenKeyboard};
use consoleui_core::search::normalize;
use consoleui_core::{Action, sound::Sound};

use crate::appstream::{Listing, Shot};
use crate::backend::{Request, Response};

/// Tiles per row and rows on the screen; only these reach the UI, the catalog has thousands.
pub const COLUMNS: usize = 7;
pub const ROWS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Games,
    Apps,
    Installed,
}

impl Section {
    pub const ALL: [Section; 3] = [Self::Games, Self::Apps, Self::Installed];

    fn contains(self, listing: &Listing) -> bool {
        match self {
            Self::Games => listing.game,
            Self::Apps => !listing.game,
            Self::Installed => listing.installed,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Changed(Sound),
    Send(Request),
    /// Start the installed app
    Open(String),
    Quit,
}

/// What just finished, for a toast and for the shell's library.
#[derive(Debug, PartialEq, Eq)]
pub enum Finished {
    Nothing,
    Installed(String),
    Removed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Cancel,
    Remove(String),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Job {
    pub percent: Option<u8>,
    pub removing: bool,
}

/// The buttons of the detail page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailButton {
    Install,
    Open,
    Remove,
}

impl DetailButton {
    pub fn label(self) -> &'static str {
        match self {
            Self::Install => tr("Install"),
            Self::Open => tr("Open"),
            Self::Remove => tr("Remove"),
        }
    }
}

/// What has the focus on the detail page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Buttons,
    Shots,
    Text,
}

pub struct Page {
    listings: Vec<Listing>,
    pub loading: bool,
    pub error: Option<String>,
    section: usize,
    pub query: String,
    /// The on-screen keyboard is open
    pub typing: bool,
    pub keyboard: OnScreenKeyboard,
    /// Indices into `listings` for the section or search
    view: Vec<usize>,
    focused: usize,
    top_row: usize,
    /// The open detail page, index into `listings`
    detail: Option<usize>,
    button: usize,
    area: Area,
    shot: usize,
    /// The focused screenshot fills the screen
    viewing: bool,
    pub scroll: i32,
    /// How far the description can scroll, from the UI
    pub max_scroll: i32,
    jobs: HashMap<String, Job>,
    pub dialog: Option<Dialog<Confirm>>,
}

impl Page {
    pub fn new() -> Self {
        Self {
            listings: Vec::new(),
            loading: true,
            error: None,
            section: 0,
            query: String::new(),
            typing: false,
            keyboard: OnScreenKeyboard::default(),
            view: Vec::new(),
            focused: 0,
            top_row: 0,
            detail: None,
            button: 0,
            area: Area::Buttons,
            shot: 0,
            viewing: false,
            scroll: 0,
            max_scroll: 0,
            jobs: HashMap::new(),
            dialog: None,
        }
    }

    pub fn section(&self) -> Option<usize> {
        self.query.is_empty().then_some(self.section)
    }

    pub fn total(&self) -> usize {
        self.view.len()
    }

    /// The tiles on the screen.
    pub fn visible(&self) -> impl Iterator<Item = &Listing> {
        self.view.iter().skip(self.top_row * COLUMNS).take(ROWS * COLUMNS).map(|&i| &self.listings[i])
    }

    /// Index of the first tile on the screen.
    pub fn first_visible(&self) -> usize {
        self.top_row * COLUMNS
    }

    pub fn visible_focus(&self) -> usize {
        self.focused - self.top_row * COLUMNS
    }

    pub fn detail(&self) -> Option<&Listing> {
        self.detail.map(|i| &self.listings[i])
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.jobs.get(id).copied()
    }

    pub fn buttons(&self) -> Vec<DetailButton> {
        match self.detail() {
            Some(listing) if self.jobs.contains_key(&listing.id) => Vec::new(),
            Some(listing) if listing.installed => vec![DetailButton::Open, DetailButton::Remove],
            Some(_) => vec![DetailButton::Install],
            None => Vec::new(),
        }
    }

    pub fn button(&self) -> usize {
        self.button
    }

    pub fn area(&self) -> Area {
        self.area
    }

    pub fn shots(&self) -> &[Shot] {
        self.detail().map_or(&[], |listing| &listing.shots)
    }

    pub fn shot(&self) -> usize {
        self.shot
    }

    pub fn viewing(&self) -> bool {
        self.viewing
    }

    /// A screenshot that cannot be loaded leaves the page.
    pub fn drop_shot(&mut self, url: &str) {
        for listing in &mut self.listings {
            listing.shots.retain(|shot| shot.url != url);
        }
        let count = self.shots().len();
        self.shot = self.shot.min(count.saturating_sub(1));
        if count == 0 {
            self.viewing = false;
            if self.area == Area::Shots {
                self.area = Area::Buttons;
            }
        }
    }

    /// A click on a screenshot.
    pub fn view_shot(&mut self, index: usize) {
        self.area = Area::Shots;
        self.shot = index.min(self.shots().len().saturating_sub(1));
        self.viewing = true;
    }

    fn rebuild(&mut self) {
        let keep = self.view.get(self.focused).copied();
        self.view = if self.query.is_empty() {
            let section = Section::ALL[self.section];
            (0..self.listings.len()).filter(|&i| section.contains(&self.listings[i])).collect()
        } else {
            search(&self.listings, &self.query)
        };
        self.focused = keep.and_then(|i| self.view.iter().position(|&v| v == i)).unwrap_or(0);
        self.follow_focus();
    }

    fn follow_focus(&mut self) {
        let row = self.focused / COLUMNS;
        if row < self.top_row {
            self.top_row = row;
        } else if row >= self.top_row + ROWS {
            self.top_row = row + 1 - ROWS;
        }
    }

    fn reset_view(&mut self) {
        self.focused = 0;
        self.top_row = 0;
        self.view.clear();
        self.rebuild();
    }

    /// Takes over what the backend reports.
    pub fn receive(&mut self, response: Response) -> Finished {
        match response {
            Response::Catalog(Ok(mut listings)) => {
                sort(&mut listings);
                self.listings = listings;
                self.loading = false;
                self.rebuild();
            }
            Response::Catalog(Err(err)) => {
                self.loading = false;
                self.error = Some(err);
            }
            Response::Progress(id, percent) => {
                self.jobs.entry(id).or_insert(Job { percent, removing: false }).percent = percent;
            }
            Response::Installed(id) => return self.finish(&id, true),
            Response::Removed(id) => return self.finish(&id, false),
            Response::Failed(id, err) => {
                let removing = self.jobs.remove(&id).is_some_and(|job| job.removing);
                let name = self.name_of(&id);
                let title = if removing {
                    trf("{} could not be removed", &[&name])
                } else {
                    trf("{} could not be installed", &[&name])
                };
                self.button = 0;
                self.dialog = Some(Dialog::alert(title, err, tr("OK"), Confirm::Cancel));
            }
        }
        Finished::Nothing
    }

    fn name_of(&self, id: &str) -> String {
        self.listings.iter().find(|l| l.id == id).map_or_else(|| id.to_string(), |l| l.name.clone())
    }

    fn finish(&mut self, id: &str, installed: bool) -> Finished {
        self.jobs.remove(id);
        if let Some(listing) = self.listings.iter_mut().find(|l| l.id == id) {
            listing.installed = installed;
        }
        self.button = 0;
        self.rebuild();
        let name = self.name_of(id);
        if installed { Finished::Installed(name) } else { Finished::Removed(name) }
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        if self.dialog.is_some() {
            return self.apply_dialog(action);
        }
        if self.typing {
            return self.apply_keyboard(action);
        }
        if self.detail.is_some() {
            return self.apply_detail(action);
        }
        self.apply_grid(action)
    }

    fn apply_dialog(&mut self, action: Action) -> Outcome {
        let Some(dialog) = self.dialog.as_mut() else { return Outcome::Nothing };
        match dialog.apply(action) {
            DialogOutcome::None => Outcome::Changed(Sound::Move),
            DialogOutcome::Dismissed => {
                self.dialog = None;
                Outcome::Changed(Sound::Back)
            }
            DialogOutcome::Chosen(choice) => {
                self.dialog = None;
                match choice {
                    Confirm::Cancel => Outcome::Changed(Sound::Back),
                    Confirm::Quit => Outcome::Quit,
                    Confirm::Remove(id) => {
                        self.jobs.insert(id.clone(), Job { percent: None, removing: true });
                        Outcome::Send(Request::Remove(id))
                    }
                }
            }
        }
    }

    fn apply_keyboard(&mut self, action: Action) -> Outcome {
        if action == Action::Back {
            self.typing = false;
            return Outcome::Changed(Sound::Back);
        }
        match self.keyboard.apply(action) {
            KeyboardEvent::None => return Outcome::Nothing,
            KeyboardEvent::Moved | KeyboardEvent::Shift => return Outcome::Changed(Sound::Move),
            KeyboardEvent::Type(c) => self.query.push(c),
            KeyboardEvent::Space => self.query.push(' '),
            KeyboardEvent::Backspace => {
                self.query.pop();
            }
            KeyboardEvent::Done => self.typing = false,
        }
        self.reset_view();
        Outcome::Changed(Sound::Confirm)
    }

    /// Text from a real keyboard.
    pub fn typed(&mut self, text: &str) {
        self.query.push_str(text);
        self.reset_view();
    }

    pub fn erase(&mut self) {
        self.query.pop();
        self.reset_view();
    }

    fn apply_detail(&mut self, action: Action) -> Outcome {
        let buttons = self.buttons();
        let shots = self.shots().len();
        if self.viewing {
            return self.apply_viewer(action, shots);
        }
        match (self.area, action) {
            (_, Action::Back) => {
                self.detail = None;
                return Outcome::Changed(Sound::Back);
            }
            (Area::Buttons, Action::Left) if self.button > 0 => self.button -= 1,
            (Area::Buttons, Action::Right) if self.button + 1 < buttons.len() => self.button += 1,
            (Area::Buttons, Action::Confirm) => return self.press(buttons.get(self.button).copied()),
            (Area::Buttons, Action::Down) if shots > 0 => self.area = Area::Shots,
            (Area::Shots, Action::Left) if self.shot > 0 => self.shot -= 1,
            (Area::Shots, Action::Right) if self.shot + 1 < shots => self.shot += 1,
            (Area::Shots, Action::Up) => self.area = Area::Buttons,
            (Area::Shots, Action::Confirm) => {
                self.viewing = true;
                return Outcome::Changed(Sound::Open);
            }
            (Area::Buttons | Area::Shots | Area::Text, Action::Down) if self.scroll < self.max_scroll => {
                self.area = Area::Text;
                self.scroll += 1;
            }
            (Area::Text, Action::Up) => {
                self.scroll -= 1;
                if self.scroll == 0 {
                    self.area = if shots > 0 { Area::Shots } else { Area::Buttons };
                }
            }
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Move)
    }

    fn apply_viewer(&mut self, action: Action, shots: usize) -> Outcome {
        match action {
            Action::Left if self.shot > 0 => self.shot -= 1,
            Action::Right if self.shot + 1 < shots => self.shot += 1,
            Action::Back | Action::Confirm => {
                self.viewing = false;
                return Outcome::Changed(Sound::Back);
            }
            _ => return Outcome::Nothing,
        }
        Outcome::Changed(Sound::Move)
    }

    /// The button of the detail page; a click comes here as well.
    pub fn press(&mut self, button: Option<DetailButton>) -> Outcome {
        let Some(listing) = self.detail() else { return Outcome::Nothing };
        let (id, name) = (listing.id.clone(), listing.name.clone());
        match button {
            None => Outcome::Nothing,
            Some(DetailButton::Install) => {
                self.jobs.insert(id.clone(), Job { percent: None, removing: false });
                Outcome::Send(Request::Install(id))
            }
            Some(DetailButton::Open) => Outcome::Open(id),
            Some(DetailButton::Remove) => {
                let buttons = vec![
                    DialogButton { label: tr("Cancel").to_string(), tag: Confirm::Cancel },
                    DialogButton { label: tr("Remove").to_string(), tag: Confirm::Remove(id) },
                ];
                let title = trf("Remove {}?", &[&name]);
                self.dialog = Some(Dialog::new(title, tr("Its saved data stays on this console."), buttons, 0));
                Outcome::Changed(Sound::Open)
            }
        }
    }

    fn apply_grid(&mut self, action: Action) -> Outcome {
        let last = self.view.len().saturating_sub(1);
        let column = self.focused % COLUMNS;
        match action {
            Action::Left if column > 0 => self.focused -= 1,
            Action::Right if column + 1 < COLUMNS && self.focused < last => self.focused += 1,
            Action::Up if self.focused >= COLUMNS => self.focused -= COLUMNS,
            // Into a shorter last row: the last tile
            Action::Down if self.focused / COLUMNS < last / COLUMNS => {
                self.focused = (self.focused + COLUMNS).min(last);
            }
            Action::PrevTab | Action::NextTab => return self.switch_tab(action == Action::NextTab),
            Action::Search => {
                self.typing = true;
                self.keyboard.reset_focus();
                return Outcome::Changed(Sound::Open);
            }
            Action::Confirm => return self.open_focused(),
            Action::Back if !self.query.is_empty() => {
                self.query.clear();
                self.reset_view();
                return Outcome::Changed(Sound::Back);
            }
            Action::Back if self.jobs.is_empty() => return Outcome::Quit,
            Action::Back => {
                let buttons = vec![
                    DialogButton { label: tr("Stay").to_string(), tag: Confirm::Cancel },
                    DialogButton { label: tr("Quit").to_string(), tag: Confirm::Quit },
                ];
                let text = tr("Quitting the store stops what is being installed.");
                self.dialog = Some(Dialog::new(tr("Quit the store?"), text, buttons, 0));
                return Outcome::Changed(Sound::Open);
            }
            _ => return Outcome::Nothing,
        }
        self.follow_focus();
        Outcome::Changed(Sound::Move)
    }

    fn switch_tab(&mut self, forward: bool) -> Outcome {
        let next = if forward { self.section + 1 } else { self.section.wrapping_sub(1) };
        if next >= Section::ALL.len() {
            return Outcome::Nothing;
        }
        self.go_to_section(next);
        Outcome::Changed(Sound::Move)
    }

    /// A click on a tab comes here as well.
    pub fn go_to_section(&mut self, section: usize) {
        self.section = section;
        self.query.clear();
        self.reset_view();
    }

    /// A click on a tile: its position among the visible ones.
    pub fn focus_visible(&mut self, index: usize) {
        self.focused = (self.top_row * COLUMNS + index).min(self.view.len().saturating_sub(1));
    }

    fn open_focused(&mut self) -> Outcome {
        let Some(&index) = self.view.get(self.focused) else { return Outcome::Nothing };
        self.detail = Some(index);
        self.button = 0;
        self.area = Area::Buttons;
        self.shot = 0;
        self.scroll = 0;
        Outcome::Changed(Sound::Open)
    }
}

/// Controller apps first, then those with a verified publisher, then by name.
fn sort(listings: &mut [Listing]) {
    listings.sort_by_cached_key(|l| (!l.gamepad, !l.verified, normalize(&l.name)));
}

/// Name matches first, then the summary; every word must occur.
fn search(listings: &[Listing], query: &str) -> Vec<usize> {
    let query = normalize(query);
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let rank = |listing: &Listing| {
        let name = normalize(&listing.name);
        let all = format!("{name} {}", normalize(&listing.summary));
        if name.starts_with(&query) {
            Some(0)
        } else if words.iter().all(|w| name.contains(w)) {
            Some(1)
        } else {
            words.iter().all(|w| all.contains(w)).then_some(2)
        }
    };
    let mut found: Vec<(u8, usize)> =
        listings.iter().enumerate().filter_map(|(i, l)| rank(l).map(|r| (r, i))).collect();
    found.sort();
    found.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(index: usize, game: bool) -> Listing {
        Listing {
            id: format!("id{index:03}"),
            name: format!("App {index:03}"),
            game,
            installed: index == 3,
            ..Listing::default()
        }
    }

    fn page(games: usize) -> Page {
        let mut page = Page::new();
        let mut listings: Vec<Listing> = (0..games).map(|i| listing(i, true)).collect();
        listings.push(listing(900, false));
        page.receive(Response::Catalog(Ok(listings)));
        page
    }

    #[test]
    fn sections_split_the_catalog() {
        let mut page = page(10);
        assert_eq!(page.total(), 10);
        page.apply(Action::NextTab);
        assert_eq!(page.total(), 1);
        page.apply(Action::NextTab);
        assert_eq!(page.visible().next().map(|l| l.id.as_str()), Some("id003"));
        assert_eq!(page.apply(Action::NextTab), Outcome::Nothing);
    }

    #[test]
    fn the_grid_scrolls_by_rows_and_only_shows_the_screen() {
        let mut page = page(COLUMNS * 5);
        assert_eq!(page.visible().count(), COLUMNS * ROWS);
        assert_eq!(page.first_visible(), 0);
        for _ in 0..3 {
            page.apply(Action::Down);
        }
        assert_eq!(page.first_visible(), COLUMNS);
        assert_eq!(page.visible_focus(), COLUMNS * 2);
        assert_eq!(page.visible().next().map(|l| l.name.as_str()), Some("App 007"));
    }

    #[test]
    fn down_into_a_short_last_row_lands_on_the_last_tile() {
        let mut page = page(COLUMNS + 2);
        for _ in 0..4 {
            page.apply(Action::Right);
        }
        page.apply(Action::Down);
        assert_eq!(page.visible_focus(), COLUMNS + 1);
        assert_eq!(page.apply(Action::Down), Outcome::Nothing);
    }

    #[test]
    fn installing_shows_progress_and_then_the_other_buttons() {
        let mut page = page(2);
        page.apply(Action::Confirm);
        assert_eq!(page.buttons(), [DetailButton::Install]);
        assert_eq!(page.apply(Action::Confirm), Outcome::Send(Request::Install("id000".into())));
        assert!(page.buttons().is_empty());
        page.receive(Response::Progress("id000".into(), Some(40)));
        assert_eq!(page.job("id000").and_then(|j| j.percent), Some(40));
        assert_eq!(page.receive(Response::Installed("id000".into())), Finished::Installed("App 000".into()));
        assert_eq!(page.buttons(), [DetailButton::Open, DetailButton::Remove]);
        assert_eq!(page.apply(Action::Confirm), Outcome::Open("id000".into()));
    }

    #[test]
    fn removing_asks_first() {
        let mut page = page(5);
        page.apply(Action::NextTab);
        page.apply(Action::NextTab);
        page.apply(Action::Confirm);
        page.apply(Action::Right);
        page.apply(Action::Confirm);
        assert!(page.dialog.is_some());
        assert_eq!(page.apply(Action::Confirm), Outcome::Changed(Sound::Back), "Cancel has the focus");
        assert_eq!(page.job("id003"), None);
        page.apply(Action::Confirm);
        page.apply(Action::Right);
        assert_eq!(page.apply(Action::Confirm), Outcome::Send(Request::Remove("id003".into())));
        assert!(page.job("id003").is_some_and(|job| job.removing));
        assert_eq!(page.receive(Response::Removed("id003".into())), Finished::Removed("App 003".into()));
        assert_eq!(page.total(), 0, "the Installed section is empty now");
    }

    #[test]
    fn a_failure_is_shown_and_the_buttons_come_back() {
        let mut page = page(1);
        page.apply(Action::Confirm);
        page.apply(Action::Confirm);
        page.receive(Response::Failed("id000".into(), "No network".into()));
        assert_eq!(page.dialog.as_ref().map(|d| d.text.as_str()), Some("No network"));
        page.apply(Action::Confirm);
        assert!(page.dialog.is_none());
        assert_eq!(page.buttons(), [DetailButton::Install]);
    }

    #[test]
    fn search_finds_across_sections_and_back_leaves_it() {
        let mut page = page(3);
        page.typed("900");
        assert_eq!(page.section(), None);
        assert_eq!(page.visible().next().map(|l| l.id.as_str()), Some("id900"));
        page.apply(Action::Back);
        assert_eq!(page.section(), Some(0));
        assert_eq!(page.total(), 3);
    }

    #[test]
    fn quitting_during_an_install_asks_first() {
        let mut page = page(1);
        assert_eq!(page.apply(Action::Back), Outcome::Quit);
        let mut page = self::page(1);
        page.apply(Action::Confirm);
        page.apply(Action::Confirm);
        page.apply(Action::Back);
        assert_eq!(page.apply(Action::Back), Outcome::Changed(Sound::Open));
        page.apply(Action::Right);
        assert_eq!(page.apply(Action::Confirm), Outcome::Quit);
    }

    fn page_with_shots() -> Page {
        let mut page = Page::new();
        let shot = |url: &str| Shot { url: url.into(), caption: String::new() };
        let listing = Listing { shots: vec![shot("a"), shot("b")], ..listing(1, true) };
        page.receive(Response::Catalog(Ok(vec![listing])));
        page.apply(Action::Confirm);
        page
    }

    #[test]
    fn down_goes_from_the_buttons_to_the_screenshots_and_the_text() {
        let mut page = page_with_shots();
        page.max_scroll = 2;
        page.apply(Action::Down);
        assert_eq!(page.area(), Area::Shots);
        page.apply(Action::Right);
        assert_eq!(page.shot(), 1);
        assert_eq!(page.apply(Action::Right), Outcome::Nothing);
        page.apply(Action::Down);
        assert_eq!((page.area(), page.scroll), (Area::Text, 1));
        page.apply(Action::Up);
        assert_eq!((page.area(), page.scroll), (Area::Shots, 0));
        page.apply(Action::Up);
        assert_eq!(page.area(), Area::Buttons);
    }

    #[test]
    fn the_full_view_pages_through_and_back_returns_to_the_details() {
        let mut page = page_with_shots();
        page.apply(Action::Down);
        page.apply(Action::Confirm);
        assert!(page.viewing());
        page.apply(Action::Right);
        assert_eq!(page.shot(), 1);
        page.apply(Action::Back);
        assert!(!page.viewing());
        assert!(page.detail().is_some());
    }

    #[test]
    fn shots_that_fail_leave_the_page() {
        let mut page = page_with_shots();
        page.apply(Action::Down);
        page.apply(Action::Right);
        page.drop_shot("b");
        assert_eq!(page.shot(), 0);
        page.drop_shot("a");
        assert_eq!(page.area(), Area::Buttons);
        assert_eq!(page.apply(Action::Down), Outcome::Nothing);
    }

    #[test]
    fn controller_apps_come_first() {
        let mut listings = vec![listing(1, true), Listing { gamepad: true, ..listing(2, true) }];
        sort(&mut listings);
        assert_eq!(listings[0].id, "id002");
    }
}
