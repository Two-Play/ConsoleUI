fn main() {
    // German comes from the shared catalog, as in the shell (Story 6.6)
    consoleui_sdk::build::compile_translated("ui/main.slint", "../../lang/de/LC_MESSAGES/consoleui.po");
}
