//! The shell's service for apps on the session bus (Story 8.7): what an app in a Flatpak sandbox cannot
//! read from the profile files, and from any language. The shell answers it in `src/service.rs`,
//! the SDK reads it in `consoleui_app::session`.
//!
//! Properties: `AccentColor` (`#rrggbb`), `Animations`, `Sounds`, `ProfileName`.
//! Methods: `Toast(title, text)`.

pub const NAME: &str = "org.consoleui.Shell1";
pub const PATH: &str = "/org/consoleui/Shell1";

/// `#rrggbb`
pub fn color_to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn color_from_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#').filter(|digits| digits.len() == 6 && digits.is_ascii())?;
    let part = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([part(0)?, part(2)?, part(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_survive_the_round_trip() {
        assert_eq!(color_to_hex([0x5a, 0xa9, 0xff]), "#5aa9ff");
        assert_eq!(color_from_hex("#5aa9ff"), Some([0x5a, 0xa9, 0xff]));
        assert_eq!(color_from_hex("5aa9ff"), None);
        assert_eq!(color_from_hex("#5aa9f"), None);
        assert_eq!(color_from_hex("#5aa9fg"), None);
        assert_eq!(color_from_hex("#5aa9fä"), None);
    }
}
