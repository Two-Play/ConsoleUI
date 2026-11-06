use std::path::Path;
use std::{env, fs};

fn main() {
    // German for `@tr()` comes from the shared catalog, as in the shell (Story 6.6). Slint looks for
    // a file named after the package, so it gets a copy under that name.
    let catalog = "../../lang/de/LC_MESSAGES/consoleui.po";
    println!("cargo:rerun-if-changed={catalog}");
    let lang = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("lang");
    let target = lang.join("de/LC_MESSAGES");
    fs::create_dir_all(&target).expect("no directory for the catalog");
    fs::copy(catalog, target.join("consoleui-store.po")).expect("cannot copy the catalog");
    let config = consoleui_ui::compiler_config()
        .with_bundled_translations(lang)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint build failed");
}
