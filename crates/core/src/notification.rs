//! Messages from other programs (`org.freedesktop.Notifications`, e.g. `notify-send`), as the
//! shell shows them: a title and a short plain text.

/// Longest text of a toast, in characters
const MAX_TEXT: usize = 140;

/// What a program sent, ready for a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub title: String,
    pub text: String,
}

impl Incoming {
    /// Title and text of a notification; `None` if there is nothing to show. Without a summary the
    /// program's name is the title.
    pub fn new(app_name: &str, summary: &str, body: &str) -> Option<Self> {
        let summary = summary.trim();
        let text = shorten(&plain_text(body));
        let title = if summary.is_empty() { app_name.trim() } else { summary };
        if title.is_empty() && text.is_empty() {
            return None;
        }
        // A message with only a body shows the body as its title line, below the program's name
        if title.is_empty() { None } else { Some(Self { title: title.to_string(), text }) }
    }
}

/// The body without the small HTML subset of the specification (`<b>`, `<i>`, `<u>`, `<a href>`,
/// `<img>`), with the common entities resolved and white space collapsed.
pub fn plain_text(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            '\n' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    let out = out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    let out = out.replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_TEXT - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markup_and_entities() {
        assert_eq!(plain_text("<b>Done</b>: 3 &amp; 4 files\n<a href=\"x\">open</a>"), "Done: 3 & 4 files open");
        assert_eq!(plain_text("a &lt;b&gt; c"), "a <b> c");
        assert_eq!(plain_text("  "), "");
    }

    #[test]
    fn title_falls_back_to_the_program_and_empty_messages_are_dropped() {
        let message = Incoming::new("steam", "Download finished", "Portal 2 is <i>ready</i>").unwrap();
        assert_eq!(message, Incoming { title: "Download finished".into(), text: "Portal 2 is ready".into() });
        assert_eq!(Incoming::new("Discord", "", "hi").unwrap().title, "Discord");
        assert_eq!(Incoming::new("", "", ""), None);
        assert_eq!(Incoming::new("", "", "only a body"), None);
    }

    #[test]
    fn long_texts_are_shortened() {
        let long = "word ".repeat(100);
        let message = Incoming::new("a", "t", &long).unwrap();
        assert!(message.text.chars().count() <= MAX_TEXT && message.text.ends_with('…'));
    }
}
