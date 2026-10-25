//! Suche über die App-Titel, unempfindlich gegen Groß-/Kleinschreibung und Umlaute.

use crate::library::AppEntry;

/// Kleinbuchstaben, Akzente und Umlaute auf die Grundbuchstaben zurückgeführt („Échoes“ → „echoes“).
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' | 'à' | 'á' | 'â' | 'ã' | 'å' => out.push('a'),
            'ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'ø' => out.push('o'),
            'ü' | 'ù' | 'ú' | 'û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'ß' => out.push_str("ss"),
            c => out.push(c),
        }
    }
    out
}

/// Kleinerer Wert = besserer Treffer; `None` = kein Treffer. Jedes Suchwort muss vorkommen.
fn score(query_words: &[String], title: &str) -> Option<u32> {
    let title = normalize(title);
    let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    query_words.iter().try_fold(0, |worst, query| {
        let score = if title.starts_with(query.as_str()) {
            0
        } else if words.iter().any(|w| w.starts_with(query.as_str())) {
            1
        } else if title.contains(query.as_str()) {
            2
        } else {
            return None;
        };
        Some(score.max(worst))
    })
}

/// Indizes der passenden Einträge, beste Treffer zuerst; bei gleicher Güte bleibt die Reihenfolge
/// von `entries` (also „zuletzt genutzt“) erhalten. Eine leere Suche liefert alle Einträge.
pub fn search(entries: &[AppEntry], query: &str) -> Vec<usize> {
    let words: Vec<String> = normalize(query).split_whitespace().map(str::to_string).collect();
    let mut hits: Vec<(u32, usize)> =
        entries.iter().enumerate().filter_map(|(i, e)| score(&words, &e.title).map(|s| (s, i))).collect();
    hits.sort_by_key(|&(score, _)| score);
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::dummy_library;

    fn titles(entries: &[AppEntry], query: &str) -> Vec<String> {
        search(entries, query).into_iter().map(|i| entries[i].title.clone()).collect()
    }

    fn library(names: &[&str]) -> Vec<AppEntry> {
        let mut entries = dummy_library(names.len());
        for (entry, name) in entries.iter_mut().zip(names) {
            entry.title = name.to_string();
        }
        entries
    }

    #[test]
    fn normalizes_umlauts_and_case() {
        assert_eq!(normalize("Grüße ÄÖÜ Échoes"), "grusse aou echoes");
    }

    #[test]
    fn ranks_prefix_before_word_before_substring() {
        let entries = library(&["Grand Theft Auto V", "Theft Club", "Notheft", "Fallout 4"]);
        assert_eq!(titles(&entries, "theft"), ["Theft Club", "Grand Theft Auto V", "Notheft"]);
    }

    #[test]
    fn all_words_must_match() {
        let entries = library(&["Red Dead Redemption 2", "Dead Space", "Fallout 76"]);
        assert_eq!(titles(&entries, "dead red"), ["Red Dead Redemption 2"]);
        assert_eq!(titles(&entries, "galaxy"), Vec::<String>::new());
    }

    #[test]
    fn empty_query_keeps_order() {
        let entries = library(&["b", "a"]);
        assert_eq!(titles(&entries, "  "), ["b", "a"]);
    }

    #[test]
    fn umlaut_insensitive_match() {
        let entries = library(&["Bildbetrachter", "Mediathek für Filme"]);
        assert_eq!(titles(&entries, "fur"), ["Mediathek für Filme"]);
    }
}
