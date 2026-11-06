//! Reads the AppStream catalog that flatpak keeps on disk for a remote. The file is large (about
//! 50 MB for Flathub), so it is streamed instead of built into a tree.

use std::path::{Path, PathBuf};

use consoleui_core::desktop::wanted_categories;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// How many screenshots of an app are shown.
const MAX_SHOTS: usize = 6;
/// The width that suits the detail page best; the catalog has several sizes of each image.
const SHOT_WIDTH: i64 = 1248;

/// A screenshot on Flathub's server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub url: String,
    pub caption: String,
}

/// An app of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub developer: String,
    pub icon: Option<PathBuf>,
    pub shots: Vec<Shot>,
    pub game: bool,
    /// The app says it can be used with a controller
    pub gamepad: bool,
    /// Flathub verified that the publisher owns the app
    pub verified: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Id,
    Name,
    Summary,
    Developer,
    DeveloperName,
    Description,
    Paragraph,
    Item,
    Category,
    Control,
    Icon,
    Value,
    Screenshots,
    Screenshot,
    Image,
    Caption,
    Other,
}

impl Tag {
    fn of(name: &[u8]) -> Self {
        match name {
            b"id" => Self::Id,
            b"name" => Self::Name,
            b"summary" => Self::Summary,
            b"developer" => Self::Developer,
            b"developer_name" => Self::DeveloperName,
            b"description" => Self::Description,
            b"p" => Self::Paragraph,
            b"li" => Self::Item,
            b"category" => Self::Category,
            b"control" => Self::Control,
            b"icon" => Self::Icon,
            b"value" => Self::Value,
            b"screenshots" => Self::Screenshots,
            b"screenshot" => Self::Screenshot,
            b"image" => Self::Image,
            b"caption" => Self::Caption,
            _ => Self::Other,
        }
    }
}

/// A text with a translation: the translation wins.
#[derive(Default)]
struct Localized {
    plain: String,
    translated: String,
}

impl Localized {
    fn set(&mut self, text: String, translated: bool) {
        *(if translated { &mut self.translated } else { &mut self.plain }) = text;
    }

    fn add(&mut self, text: String, translated: bool) {
        let target = if translated { &mut self.translated } else { &mut self.plain };
        if !target.is_empty() {
            target.push_str("\n\n");
        }
        target.push_str(&text);
    }

    fn take(self) -> String {
        if self.translated.is_empty() { self.plain } else { self.translated }
    }
}

#[derive(Default)]
struct Draft {
    id: String,
    name: Localized,
    summary: Localized,
    description: Localized,
    developer: String,
    developer_name: Localized,
    icon: Option<String>,
    shots: Vec<Shot>,
    /// Sizes of the screenshot being read
    images: Vec<(i64, String)>,
    caption: Localized,
    categories: Vec<String>,
    gamepad: bool,
    verified: bool,
}

impl Draft {
    /// The end of a screenshot: picks the size that suits the detail page.
    fn finish_shot(&mut self) {
        let images = std::mem::take(&mut self.images);
        let caption = std::mem::take(&mut self.caption).take();
        let best = images.into_iter().min_by_key(|(width, _)| (width - SHOT_WIDTH).abs());
        if let Some((_, url)) = best.filter(|_| self.shots.len() < MAX_SHOTS) {
            self.shots.push(Shot { url, caption });
        }
    }

    fn finish(self, icons: &Path) -> Option<Listing> {
        let name = self.name.take();
        if self.id.is_empty() || name.is_empty() || !wanted_categories(&self.categories) {
            return None;
        }
        let developer_name = self.developer_name.take();
        Some(Listing {
            id: self.id.trim_end_matches(".desktop").to_string(),
            name,
            summary: self.summary.take(),
            description: self.description.take(),
            developer: if self.developer.is_empty() { developer_name } else { self.developer },
            icon: self.icon.map(|file| icons.join("128x128").join(file)),
            shots: self.shots,
            game: self.categories.iter().any(|c| c == "Game"),
            gamepad: self.gamepad,
            verified: self.verified,
            installed: false,
        })
    }
}

fn attribute(element: &BytesStart, key: &[u8]) -> Option<String> {
    let found = element.attributes().flatten().find(|a| a.key.as_ref() == key)?;
    Some(String::from_utf8_lossy(&found.value).into_owned())
}

/// Whether a text in the language `lang` (`xml:lang`) counts: `Some(translated)`, or `None` for
/// another language. English is the untagged text.
fn language_of(element: &BytesStart, wanted: &str) -> Option<bool> {
    match attribute(element, b"xml:lang") {
        None => Some(false),
        Some(lang) if !wanted.is_empty() && lang.split('_').next() == Some(wanted) => Some(true),
        Some(_) => None,
    }
}

fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The apps in the catalog that suit the console, in the language `lang` (`""` for English).
/// `icons` is the directory next to the catalog that holds `128x128/<file>`.
pub fn parse(xml: &[u8], icons: &Path, lang: &str) -> Vec<Listing> {
    let mut reader = Reader::from_reader(xml);
    let mut listings = Vec::new();
    let mut draft: Option<Draft> = None;
    // Tags below the component; [0] is the direct child
    let mut stack: Vec<Tag> = Vec::new();
    let mut text = String::new();
    // Language of the element whose text is collected; `None` skips it
    let mut language: Option<bool> = None;
    let mut verified_value = false;
    let mut image_width = 0;
    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(err) => {
                eprintln!("Broken catalog: {err}");
                break;
            }
        };
        match event {
            Event::Start(element) => {
                if draft.is_none() {
                    let kind = attribute(&element, b"type");
                    if element.name().as_ref() == b"component"
                        && kind.is_some_and(|k| k == "desktop-application" || k == "desktop")
                    {
                        draft = Some(Draft::default());
                        stack.clear();
                    }
                    continue;
                }
                let tag = Tag::of(element.name().as_ref());
                stack.push(tag);
                match tag {
                    Tag::Paragraph | Tag::Item | Tag::Name | Tag::Summary | Tag::DeveloperName | Tag::Caption => {
                        language = language_of(&element, lang);
                        text.clear();
                    }
                    Tag::Icon => {
                        let wanted = attribute(&element, b"type").as_deref() == Some("cached")
                            && attribute(&element, b"height").as_deref() == Some("128")
                            && attribute(&element, b"scale").is_none();
                        language = wanted.then_some(false);
                        text.clear();
                    }
                    Tag::Value => {
                        verified_value =
                            attribute(&element, b"key").as_deref() == Some("flathub::verification::verified");
                        text.clear();
                    }
                    Tag::Image => {
                        image_width = attribute(&element, b"width").and_then(|w| w.parse().ok()).unwrap_or(0);
                        text.clear();
                    }
                    Tag::Id | Tag::Category | Tag::Control => text.clear(),
                    _ => {}
                }
            }
            Event::Text(value) if draft.is_some() => {
                if let Ok(value) = value.decode() {
                    text.push_str(&value);
                }
            }
            Event::GeneralRef(entity) if draft.is_some() => match entity.resolve_char_ref() {
                Ok(Some(c)) => text.push(c),
                _ => match &*entity {
                    b"amp" => text.push('&'),
                    b"lt" => text.push('<'),
                    b"gt" => text.push('>'),
                    b"quot" => text.push('"'),
                    b"apos" => text.push('\''),
                    _ => {}
                },
            },
            Event::End(element) => {
                let Some(current) = draft.as_mut() else { continue };
                let Some(tag) = stack.pop() else {
                    // The component itself
                    if element.name().as_ref() == b"component" {
                        listings.extend(draft.take().and_then(|d| d.finish(icons)));
                    }
                    continue;
                };
                let parent = stack.first().copied();
                let top = stack.is_empty();
                match (tag, language) {
                    (Tag::Id, _) if top => current.id = tidy(&text),
                    (Tag::Name, Some(local)) if top => current.name.set(tidy(&text), local),
                    (Tag::Summary, Some(local)) if top => current.summary.set(tidy(&text), local),
                    (Tag::DeveloperName, Some(local)) if top => current.developer_name.set(tidy(&text), local),
                    (Tag::Name, Some(false)) if parent == Some(Tag::Developer) => current.developer = tidy(&text),
                    (Tag::Paragraph, Some(local)) if parent == Some(Tag::Description) => {
                        current.description.add(tidy(&text), local);
                    }
                    (Tag::Item, Some(local)) if parent == Some(Tag::Description) => {
                        current.description.add(format!("• {}", tidy(&text)), local);
                    }
                    (Tag::Icon, Some(_)) if top => current.icon = Some(tidy(&text)),
                    (Tag::Image, _) if parent == Some(Tag::Screenshots) => {
                        let url = tidy(&text);
                        if [".png", ".jpg", ".jpeg"].iter().any(|end| url.to_lowercase().ends_with(end)) {
                            current.images.push((image_width, url));
                        }
                    }
                    (Tag::Caption, Some(local)) if parent == Some(Tag::Screenshots) => {
                        current.caption.set(tidy(&text), local);
                    }
                    (Tag::Screenshot, _) if parent == Some(Tag::Screenshots) => current.finish_shot(),
                    (Tag::Category, _) => current.categories.push(tidy(&text)),
                    (Tag::Control, _) if text.trim() == "gamepad" => current.gamepad = true,
                    (Tag::Value, _) if verified_value && text.trim() == "true" => current.verified = true,
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    listings
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<components version="0.8" origin="flatpak">
  <component type="desktop-application">
    <id>org.example.Game</id>
    <name>Game</name>
    <name xml:lang="de">Spiel</name>
    <name xml:lang="fr">Jeu</name>
    <summary>Fun &amp; games</summary>
    <description>
      <p>First <em>line</em>.</p>
      <p xml:lang="fr">Premier.</p>
      <ul><li>A point</li></ul>
    </description>
    <developer id="x"><name>Example Team</name></developer>
    <supports><control>keyboard</control><control>gamepad</control></supports>
    <icon type="stock">org.example.Game</icon>
    <icon type="cached" height="64" width="64">small.png</icon>
    <icon type="cached" height="128" width="128" scale="2">double.png</icon>
    <icon type="cached" height="128" width="128">org.example.Game.png</icon>
    <categories><category>Game</category></categories>
    <screenshots>
      <screenshot type="default">
        <caption>The menu</caption>
        <caption xml:lang="de">Das Menü</caption>
        <image type="source" width="3840">https://example.org/orig.png</image>
        <image type="thumbnail" width="1248">https://example.org/1248.png</image>
        <image type="thumbnail" width="224">https://example.org/224.png</image>
      </screenshot>
      <screenshot><image type="source" width="800">https://example.org/two.jpg</image></screenshot>
      <screenshot><image type="source" width="800">https://example.org/three.svg</image></screenshot>
    </screenshots>
    <releases><release><description><p>Not this one</p></description></release></releases>
    <custom><value key="other">true</value><value key="flathub::verification::verified">true</value></custom>
  </component>
  <component type="desktop-application">
    <id>org.example.Player.desktop</id>
    <name>Player</name>
    <developer_name>Someone</developer_name>
    <categories><category>AudioVideo</category></categories>
    <custom><value key="other">true</value></custom>
  </component>
  <component type="desktop-application">
    <id>org.example.Ide</id>
    <name>IDE</name>
    <categories><category>Development</category></categories>
  </component>
  <component type="runtime">
    <id>org.example.Runtime</id>
    <name>Runtime</name>
    <categories><category>Game</category></categories>
  </component>
</components>"#;

    #[test]
    fn reads_what_the_store_shows() {
        let listings = parse(CATALOG.as_bytes(), Path::new("/icons"), "");
        assert_eq!(listings.len(), 2, "development tools and runtimes stay out");
        let game = &listings[0];
        assert_eq!(game.name, "Game");
        assert_eq!(game.summary, "Fun & games");
        assert_eq!(game.description, "First line.\n\n• A point");
        assert_eq!(game.developer, "Example Team");
        assert_eq!(game.icon.as_deref(), Some(Path::new("/icons/128x128/org.example.Game.png")));
        assert!(game.game && game.gamepad && game.verified);
        let urls: Vec<&str> = game.shots.iter().map(|shot| shot.url.as_str()).collect();
        assert_eq!(urls, ["https://example.org/1248.png", "https://example.org/two.jpg"]);
        assert_eq!(game.shots[0].caption, "The menu");
        let player = &listings[1];
        assert_eq!(player.id, "org.example.Player");
        assert_eq!(player.developer, "Someone");
        assert!(!player.game && !player.gamepad && !player.verified);
    }

    #[test]
    fn prefers_the_wanted_language() {
        let listings = parse(CATALOG.as_bytes(), Path::new("/icons"), "de");
        assert_eq!(listings[0].name, "Spiel");
        assert_eq!(listings[0].shots[0].caption, "Das Menü");
        assert_eq!(listings[1].name, "Player", "untranslated texts stay English");
    }
}
