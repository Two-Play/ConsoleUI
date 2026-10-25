//! Bildformat am Dateiinhalt erkennen. Manche `.desktop`-Dateien (z. B. von AppImage-Launchern)
//! verweisen auf Icons ohne Dateiendung, an denen Slint das Format nicht erkennen kann.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Svg,
}

/// So viele Bytes vom Dateianfang reichen für [`sniff`].
pub const SNIFF_LEN: usize = 512;

/// Erkennt PNG und JPEG an ihrer Signatur und SVG an `<svg` (ggf. hinter XML-Deklaration,
/// Kommentaren oder Doctype) im Dateianfang.
pub fn sniff(head: &[u8]) -> Option<ImageFormat> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(ImageFormat::Jpeg);
    }
    let head = &head[..head.len().min(SNIFF_LEN)];
    let text = head.strip_prefix(b"\xef\xbb\xbf").unwrap_or(head);
    let text = String::from_utf8_lossy(text);
    let trimmed = text.trim_start();
    (trimmed.starts_with('<') && trimmed.contains("<svg")).then_some(ImageFormat::Svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erkennt_png_und_jpeg_an_der_signatur() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some(ImageFormat::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10, b'J', b'F']), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn erkennt_svg_auch_hinter_deklaration_und_bom() {
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Some(ImageFormat::Svg));
        let with_decl = b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<!-- Inkscape -->\n<svg width=\"48\">";
        assert_eq!(sniff(with_decl), Some(ImageFormat::Svg));
        assert_eq!(sniff(b"  \n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\">\n<svg>"), Some(ImageFormat::Svg));
    }

    #[test]
    fn unbekanntes_bleibt_none() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b"<html><body>kein svg</body></html>"), None);
        assert_eq!(sniff(b"svg ohne spitze Klammer"), None);
        // PNG-Signatur unvollständig
        assert_eq!(sniff(b"\x89PNG"), None);
    }
}
