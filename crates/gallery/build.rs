fn main() {
    slint_build::compile_with_config("ui/main.slint", consoleui_ui::compiler_config()).expect("Slint build failed");
}
