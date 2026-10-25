//! Simplified icon lookup following the freedesktop Icon Theme Specification
//! (without evaluating `index.theme`; theme, then `hicolor`, then `pixmaps`).

use std::path::{Path, PathBuf};

/// Largest variants first, so the tiles stay sharp on 4K too.
const SIZES: &[&str] = &["scalable", "512x512", "256x256", "192x192", "128x128", "96x96", "64x64", "48x48"];
/// Breeze stores icons as `apps/<size>/name.svg`.
const BREEZE_SIZES: &[&str] = &["64", "48", "32"];
const EXTENSIONS: &[&str] = &["svg", "png"];

pub fn find(name: &str, theme: &str, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let name = name.strip_suffix(".png").or_else(|| name.strip_suffix(".svg")).unwrap_or(name);

    let mut themes = vec![theme];
    if theme != "hicolor" {
        themes.push("hicolor");
    }
    for theme in themes {
        for dir in data_dirs {
            let base = dir.join("icons").join(theme);
            if !base.is_dir() {
                continue;
            }
            let candidates = SIZES
                .iter()
                .map(|size| base.join(size).join("apps"))
                .chain(BREEZE_SIZES.iter().map(|size| base.join("apps").join(size)));
            if let Some(found) = candidates.into_iter().find_map(|dir| with_extension(&dir, name)) {
                return Some(found);
            }
        }
    }
    data_dirs.iter().find_map(|dir| with_extension(&dir.join("pixmaps"), name))
}

fn with_extension(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|p| p.is_file())
}
