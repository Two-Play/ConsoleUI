//! The ConsoleUI design system: the Slint components, design tokens and shared types of the shell,
//! usable by every app of the ecosystem so they all look and feel the same.
//!
//! The components live in `ui/*.slint` and are imported through the library name `@consoleui`:
//!
//! ```slint
//! import { Theme, GlassPanel, Button, HintBar } from "@consoleui/std.slint";
//! ```
//!
//! An app lists this crate as a dependency and as a build dependency (feature `build`) and compiles
//! its UI in `build.rs` with the configuration from [`compiler_config`]:
//!
//! ```ignore
//! let config = consoleui_ui::compiler_config().with_bundled_translations("../../lang");
//! slint_build::compile_with_config("ui/main.slint", config).unwrap();
//! ```

use std::path::PathBuf;

/// Name under which the components are imported (`@consoleui/...`).
pub const LIBRARY_NAME: &str = "consoleui";

/// Directory with the `.slint` files of the library.
pub fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// Slint compiler configuration that resolves `@consoleui/...` imports. Add the app's own
/// settings (translations, style) on the returned value.
#[cfg(feature = "build")]
pub fn compiler_config() -> slint_build::CompilerConfiguration {
    slint_build::CompilerConfiguration::new()
        .with_library_paths(std::collections::HashMap::from([(LIBRARY_NAME.to_string(), ui_dir())]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `std.slint` must re-export every component file, so apps need one import only.
    #[test]
    fn std_exports_every_component() {
        let std = std::fs::read_to_string(ui_dir().join("std.slint")).unwrap();
        for entry in std::fs::read_dir(ui_dir()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".slint").filter(|s| *s != "std") {
                assert!(std.contains(&format!("\"{stem}.slint\"")), "std.slint does not re-export {name}");
            }
        }
    }
}
