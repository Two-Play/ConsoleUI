//! Zustand der Shell: Einträge, Fokus, Historie und die Verbindung zur UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use consoleui_core::{Action, AppEntry, Carousel, History, launch};
use slint::{Image, Model, ModelRc, VecModel};

use crate::{AppWindow, AppTile, tiles};

pub struct Shell {
    entries: Vec<AppEntry>,
    tiles: Rc<VecModel<AppTile>>,
    carousel: Carousel,
    history: History,
    history_path: Option<PathBuf>,
    /// Hero-Bilder werden erst beim Fokussieren geladen und dann behalten.
    heroes: HashMap<String, Image>,
}

impl Shell {
    pub fn new(ui: &AppWindow, mut entries: Vec<AppEntry>, history: History, history_path: Option<PathBuf>) -> Self {
        let today = tiles::today();
        let tile_list: Vec<AppTile> =
            entries.iter_mut().enumerate().map(|(i, entry)| tiles::to_tile(entry, i, today)).collect();
        let tiles = Rc::new(VecModel::from(tile_list));
        ui.set_tiles(ModelRc::from(tiles.clone()));

        let mut shell = Self {
            carousel: Carousel::new(entries.len()),
            entries,
            tiles,
            history,
            history_path,
            heroes: HashMap::new(),
        };
        shell.show_hero(ui);
        shell
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn apply(&mut self, ui: &AppWindow, action: Action) {
        if ui.get_launching() {
            return;
        }
        match action {
            Action::Confirm => self.launch(ui),
            _ => {
                if self.carousel.apply(action) {
                    ui.set_focused(self.carousel.focused() as i32);
                    self.show_hero(ui);
                }
            }
        }
    }

    /// Lädt das Hero-Bild in die gerade unsichtbare Ebene und blendet dann um.
    fn show_hero(&mut self, ui: &AppWindow) {
        let hero = self.hero(self.carousel.focused()).unwrap_or_default();
        let front_a = !ui.get_hero_front_a();
        if front_a {
            ui.set_hero_a(hero);
        } else {
            ui.set_hero_b(hero);
        }
        ui.set_hero_front_a(front_a);
    }

    fn hero(&mut self, index: usize) -> Option<Image> {
        let entry = self.entries.get(index)?;
        let path = entry.artwork.hero.as_deref()?;
        if !self.heroes.contains_key(&entry.id) {
            self.heroes.insert(entry.id.clone(), tiles::load(path)?);
        }
        self.heroes.get(&entry.id).cloned()
    }

    fn launch(&mut self, ui: &AppWindow) {
        let index = self.carousel.focused();
        let hero = self.hero(index);
        let (Some(entry), Some(tile)) = (self.entries.get(index), self.tiles.row_data(index)) else { return };

        let started = launch::spawn(&entry.launch);
        if let Err(err) = &started {
            eprintln!("{} konnte nicht gestartet werden: {err}", entry.title);
        }
        ui.set_launch_title(tile.title.clone());
        ui.set_launch_image(hero.unwrap_or(tile.cover));
        ui.set_launch_accent(tile.accent);
        ui.set_launch_status(if started.is_ok() { "wird gestartet …" } else { "konnte nicht gestartet werden" }.into());
        ui.set_launching(true);

        if started.is_ok() {
            self.record_launch(index);
            ui.set_focused(0);
        }
    }

    /// Speichert den Start in der Historie und schiebt den Eintrag nach vorn. Das passiert unter
    /// dem Startbildschirm, deshalb sieht man die Umsortierung nicht.
    fn record_launch(&mut self, index: usize) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let mut entry = self.entries.remove(index);
        entry.last_used = Some(now);
        self.history.record(&entry.id, now);
        if let Some(path) = &self.history_path
            && let Err(err) = self.history.save(path)
        {
            eprintln!("Historie konnte nicht gespeichert werden ({}): {err}", path.display());
        }

        let mut tile = self.tiles.remove(index);
        tile.subtitle = tiles::subtitle(&entry, tiles::today()).into();
        self.entries.insert(0, entry);
        self.tiles.insert(0, tile);
        self.carousel.focus(0);
    }
}
