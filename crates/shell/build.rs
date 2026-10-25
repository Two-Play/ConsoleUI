fn main() {
    // German for `@tr()` comes from the shared catalog `lang/de/LC_MESSAGES/consoleui.po`, bundled
    // into the binary (Story 6.6). No translation context: the same messages are also used from
    // Rust (`consoleui_core::i18n`).
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("../../lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/main.slint", config).expect("Slint build failed");
}
