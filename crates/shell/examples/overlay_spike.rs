//! Spike 4.6: semi-transparent fullscreen window with STEAM_OVERLAY (set before mapping).
//! Run inside gamescope: `overlay_spike <seconds> [steam_game_appid]`

use std::error::Error;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ColormapAlloc, ConnectionExt as _, CreateGCAux, CreateWindowAux, PropMode, Rectangle, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

fn main() -> Result<(), Box<dyn Error>> {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let app_id: Option<u32> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let (depth, visual) = screen
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter().map(move |v| (d.depth, v.visual_id)))
        .next()
        .ok_or("no 32-bit visual")?;
    let colormap = conn.generate_id()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual)?;
    let window = conn.generate_id()?;
    // Premultiplied alpha: semi-transparent dark blue
    let argb = 0x99_0a_10_28u32;
    conn.create_window(
        depth,
        window,
        screen.root,
        0,
        0,
        screen.width_in_pixels,
        screen.height_in_pixels,
        0,
        WindowClass::INPUT_OUTPUT,
        visual,
        &CreateWindowAux::new().background_pixel(argb).border_pixel(0).colormap(colormap),
    )?;
    let atom = |name: &[u8]| -> Result<u32, Box<dyn Error>> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
    conn.change_property32(PropMode::REPLACE, window, atom(b"STEAM_OVERLAY")?, AtomEnum::CARDINAL, &[1])?;
    if let Some(id) = app_id {
        conn.change_property32(PropMode::REPLACE, window, atom(b"STEAM_GAME")?, AtomEnum::CARDINAL, &[id])?;
    }
    if std::env::var_os("SPIKE_EXTERNAL").is_some() {
        conn.change_property32(PropMode::REPLACE, window, atom(b"GAMESCOPE_EXTERNAL_OVERLAY")?, AtomEnum::CARDINAL, &[1])?;
    }
    conn.map_window(window)?;
    conn.flush()?;
    println!("Overlay window {window} mapped");
    // Draw real pixels so XWayland sends a buffer to gamescope
    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &CreateGCAux::new().foreground(argb))?;
    for _ in 0..(secs * 10) {
        let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
        conn.poly_fill_rectangle(window, gc, &[Rectangle { x: 0, y: 0, width: w, height: h }])?;
        conn.poly_fill_rectangle(window, gc, &[Rectangle { x: 40, y: 40, width: w / 3, height: h / 3 }])?;
        conn.flush()?;
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
