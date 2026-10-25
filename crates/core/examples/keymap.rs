//! Prints the keyboard layout our compositor would detect on this machine
//! (`cargo run -p consoleui-core --example keymap`).

fn main() {
    let (keymap, source) = consoleui_core::keymap::detect();
    println!("{} from {source:?}: {keymap:?}", keymap.describe());
}
