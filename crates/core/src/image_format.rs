//! Detects the image format from the file content. Some `.desktop` files (e.g. from AppImage
//! launchers) point to icons without a file extension, from which Slint cannot tell the format.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Svg,
}

/// This many bytes from the start of the file are enough for [`sniff`].
pub const SNIFF_LEN: usize = 512;

/// Detects PNG and JPEG by their signature and SVG by `<svg` (possibly after an XML declaration,
/// comments or a doctype) at the start of the file.
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
    fn detects_png_and_jpeg_by_signature() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Some(ImageFormat::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10, b'J', b'F']), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn detects_svg_after_declaration_and_bom() {
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Some(ImageFormat::Svg));
        let with_decl = b"\xef\xbb\xbf<?xml version=\"1.0\"?>\n<!-- Inkscape -->\n<svg width=\"48\">";
        assert_eq!(sniff(with_decl), Some(ImageFormat::Svg));
        assert_eq!(sniff(b"  \n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\">\n<svg>"), Some(ImageFormat::Svg));
    }

    #[test]
    fn unknown_stays_none() {
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b"<html><body>kein svg</body></html>"), None);
        assert_eq!(sniff(b"svg ohne spitze Klammer"), None);
        // Incomplete PNG signature
        assert_eq!(sniff(b"\x89PNG"), None);
    }
}
