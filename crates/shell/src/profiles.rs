//! Glue for the profiles (Story 5.7): which profile the shell starts with, and the
//! [`PickerView`] as the Slint struct `ProfileView`.

use std::path::PathBuf;

use consoleui_core::osk::OnScreenKeyboard;
use consoleui_core::profile_picker::PickerView;
use consoleui_core::profiles::{self, Profile, ProfilePaths, Profiles};
use consoleui_core::settings::Settings;
use slint::{Color, ModelRc, VecModel};

use crate::{ContextItem, KeyCap, ProfileTile, ProfileView};

/// The profiles as found at the start, and the one the shell starts with
pub struct ProfileStart {
    pub profiles: Profiles,
    /// `$XDG_DATA_HOME/consoleui` (`None` without a home directory: nothing is saved)
    pub root: Option<PathBuf>,
    pub active: String,
    pub settings: Settings,
}

impl ProfileStart {
    /// Loads the profile list (creating the first profile from the files of before profiles) and
    /// picks `choice` (ID or name, `--profile`) or the profile chosen last.
    pub fn load(choice: Option<&str>) -> Self {
        let root = Profiles::default_root();
        let mut profiles = root.as_deref().map(Profiles::load).unwrap_or_default();
        match &root {
            Some(root) => {
                if profiles.ensure_one(root, &profiles::default_name(), Settings::default_path().as_deref())
                    && let Err(err) = profiles.save(root)
                {
                    eprintln!("Cannot save the profiles: {err}");
                }
            }
            None => {
                profiles.add(&profiles::default_name());
            }
        }
        let chosen = choice.and_then(|c| {
            let found = profiles.find(c);
            if found.is_none() {
                eprintln!("No profile \"{c}\", starting with the last one.");
            }
            found
        });
        let active = chosen.or(profiles.last()).map(|p| p.id.clone()).unwrap_or_default();
        let settings =
            root.as_deref().map(|root| Settings::load(&ProfilePaths::new(root, &active).settings)).unwrap_or_default();
        Self { profiles, root, active, settings }
    }

    pub fn paths(&self) -> Option<ProfilePaths> {
        self.root.as_deref().map(|root| ProfilePaths::new(root, &self.active))
    }
}

fn color([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgb_u8(r, g, b)
}

/// The Slint view; `view` is `None` while the profile screen is closed (the top bar still shows
/// the active profile).
pub fn to_view(view: Option<PickerView>, active: Option<&Profile>) -> ProfileView {
    let open = view.is_some();
    let view = view.unwrap_or_default();
    let keys: Vec<KeyCap> = if view.name_open {
        OnScreenKeyboard::password()
            .keys()
            .iter()
            .zip(&view.key_labels)
            .map(|(cap, label)| KeyCap { label: label.into(), row: cap.row as i32, x: cap.x, width: cap.width })
            .collect()
    } else {
        Vec::new()
    };
    let tiles: Vec<ProfileTile> = view
        .tiles
        .iter()
        .map(|t| ProfileTile {
            name: t.name.as_str().into(),
            initial: t.initial.as_str().into(),
            color: color(t.rgb),
            current: t.current,
            add: t.add,
        })
        .collect();
    let menu: Vec<ContextItem> = view
        .menu
        .iter()
        .map(|(symbol, label)| ContextItem { symbol: (*symbol).into(), label: (*label).into() })
        .collect();
    ProfileView {
        open,
        title: view.title.into(),
        tiles: ModelRc::new(VecModel::from(tiles)),
        focused: view.focused as i32,
        menu_title: view.menu_title.into(),
        menu_items: ModelRc::new(VecModel::from(menu)),
        menu_focused: view.menu_focused as i32,
        name_open: view.name_open,
        name_title: view.name_title.into(),
        name_text: view.name_text.into(),
        keys: ModelRc::new(VecModel::from(keys)),
        key_focused: view.key_focused as i32,
        confirm_text: view.confirm.into(),
        active_name: active.map(|p| p.name.as_str()).unwrap_or_default().into(),
        active_initial: active.map(Profile::initial).unwrap_or_default().into(),
        active_color: color(active.map_or(profiles::COLORS[0], Profile::rgb)),
    }
}
