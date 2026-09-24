//! Small fuzzy matcher for tag completion, in the spirit of fzf / blink.cmp:
//! the query must appear in order as a subsequence of the candidate
//! (case-insensitive); scoring rewards prefix, consecutive, and word-start hits.

/// A scored match: higher `score` is better; `positions` are the matched char
/// indices in the candidate, for highlighting.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

const MATCH: i32 = 16;
const CONSECUTIVE: i32 = 12;
const WORD_START: i32 = 10;
const FIRST_CHAR: i32 = 20;
const GAP: i32 = -1;

/// Match `query` against `candidate`. Returns `None` if `query` isn't a
/// subsequence. An empty query matches nothing (the menu stays closed).
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return None;
    }
    let c: Vec<char> = candidate.to_lowercase().chars().collect();

    // Greedy left-to-right, but prefer a later occurrence of a char when it
    // starts a word or continues a run — cheap and good enough for short tags.
    let mut positions = Vec::with_capacity(q.len());
    let mut ci = 0;
    for &qc in &q {
        let mut found = None;
        let mut j = ci;
        while j < c.len() {
            if c[j] == qc {
                let continues = positions.last().is_some_and(|&p: &usize| p + 1 == j);
                if found.is_none() {
                    found = Some(j);
                }
                if continues || is_word_start(&c, j) {
                    found = Some(j);
                    break;
                }
            }
            j += 1;
        }
        let j = found?;
        positions.push(j);
        ci = j + 1;
    }

    let mut score = 0;
    for (k, &p) in positions.iter().enumerate() {
        score += MATCH;
        if p == 0 {
            score += FIRST_CHAR;
        }
        if is_word_start(&c, p) {
            score += WORD_START;
        }
        if k > 0 {
            let prev = positions[k - 1];
            if p == prev + 1 {
                score += CONSECUTIVE;
            } else {
                score += GAP * (p - prev - 1) as i32;
            }
        }
    }
    // Slight preference for shorter candidates (closer to what was typed).
    score -= (c.len() as i32 - q.len() as i32).max(0);
    Some(FuzzyMatch { score, positions })
}

fn is_word_start(c: &[char], i: usize) -> bool {
    i == 0 || matches!(c[i - 1], '-' | '_' | ' ' | '/' | '.' | ':')
}

/// Rank `candidates` against `query`, best first; ties break alphabetically.
pub fn rank<'a>(
    query: &str,
    candidates: impl IntoIterator<Item = &'a String>,
) -> Vec<(&'a String, FuzzyMatch)> {
    let mut out: Vec<_> = candidates
        .into_iter()
        .filter_map(|c| fuzzy_match(query, c).map(|m| (c, m)))
        .collect();
    out.sort_by(|(a, ma), (b, mb)| mb.score.cmp(&ma.score).then_with(|| a.cmp(b)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str, cands: &[&str]) -> Vec<String> {
        let owned: Vec<String> = cands.iter().map(|s| s.to_string()).collect();
        rank(query, &owned)
            .into_iter()
            .map(|(c, _)| c.clone())
            .collect()
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("wrk", "work").is_some());
        assert!(fuzzy_match("wkr", "work").is_none());
        assert!(fuzzy_match("", "work").is_none());
    }

    #[test]
    fn case_insensitive_with_positions() {
        let m = fuzzy_match("WK", "work").unwrap();
        assert_eq!(m.positions, vec![0, 3]);
    }

    #[test]
    fn prefix_beats_scattered() {
        assert_eq!(names("ho", &["photo", "home"]), vec!["home", "photo"]);
    }

    #[test]
    fn word_start_beats_mid_word() {
        // "fe" → front-end (word starts f, e) over "coffee"
        assert_eq!(
            names("fe", &["coffee", "front-end"]),
            vec!["front-end", "coffee"]
        );
    }

    #[test]
    fn consecutive_beats_gapped() {
        assert_eq!(names("urg", &["u-r-g", "urgent"]), vec!["urgent", "u-r-g"]);
    }

    #[test]
    fn prefers_word_start_occurrence_for_highlighting() {
        let m = fuzzy_match("e", "front-end").unwrap();
        assert_eq!(m.positions, vec![6]);
    }
}
