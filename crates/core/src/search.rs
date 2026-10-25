//! Search over the app titles, insensitive to case and umlauts.

use crate::collection::Collection;
use crate::library::AppEntry;

/// Lowercase, with accents and umlauts reduced to the base letters ("Échoes" → "echoes").
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

/// Smaller value = better match; `None` = no match. Every search word must occur.
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

/// Indices of the matching entries, best matches first; with equal quality the order of
/// `entries` (i.e. "recently used") is kept. An empty search returns all entries.
/// The search never finds hidden entries.
pub fn search(entries: &[AppEntry], query: &str, collection: &Collection) -> Vec<usize> {
    let words: Vec<String> = normalize(query).split_whitespace().map(str::to_string).collect();
    let mut hits: Vec<(u32, usize)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !collection.is_hidden(&e.id))
        .filter_map(|(i, e)| score(&words, &e.title).map(|s| (s, i)))
        .collect();
    hits.sort_by_key(|&(score, _)| score);
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::dummy_library;

    fn titles(entries: &[AppEntry], query: &str) -> Vec<String> {
        search(entries, query, &Collection::default()).into_iter().map(|i| entries[i].title.clone()).collect()
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
    fn hidden_entries_are_never_found() {
        let entries = library(&["Red Dead Redemption 2", "Dead Space"]);
        let mut collection = Collection::default();
        collection.hide(&entries[1].id);
        let hits: Vec<usize> = search(&entries, "dead", &collection);
        assert_eq!(hits, [0]);
        assert_eq!(search(&entries, "", &collection), [0]);
    }

    #[test]
    fn umlaut_insensitive_match() {
        let entries = library(&["Bildbetrachter", "Mediathek für Filme"]);
        assert_eq!(titles(&entries, "fur"), ["Mediathek für Filme"]);
    }
}
