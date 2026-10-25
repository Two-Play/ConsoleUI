//! Zeigt, was die Shell auf diesem System finden würde: `cargo run -p consoleui-core --example list_library`

fn main() {
    let mut entries = consoleui_core::discover_library();
    consoleui_core::sort_by_recent(&mut entries);
    for entry in &entries {
        let art = &entry.artwork;
        let flag = |p: &Option<std::path::PathBuf>, c: char| if p.is_some() { c } else { '-' };
        println!(
            "{:<28} {}{}{}{} {:?} {}",
            entry.id,
            flag(&art.cover, 'C'),
            flag(&art.hero, 'H'),
            flag(&art.logo, 'L'),
            flag(&art.icon, 'I'),
            entry.last_used,
            entry.title
        );
    }
    println!("{} Einträge", entries.len());
}
