// SPDX-License-Identifier: AGPL-3.0-only

//! "Did you mean …?" over a fixed vocabulary.
//!
//! Extracted from the recipe registry when a second caller appeared: recipe
//! names and setting keys are different vocabularies with the same problem, and
//! two rankings would drift. The rule this replaced ranked by a shared six
//! character prefix and then ALPHABETICALLY, which on a catalogue with long
//! common prefixes filled every slot with the wrong answer while the name one
//! character away never appeared.

/// How far a name may be from what was typed and still be worth printing.
///
/// A quarter of the longer of the two names: floored at 2 so a short name still
/// catches a near miss, and capped at 5 so an unrelated string prints nothing
/// rather than three wrong guesses.
const fn max_edits(len: usize) -> usize {
    match len / 4 {
        0 | 1 => 2,
        n if n > 5 => 5,
        n => n,
    }
}

/// Levenshtein distance over chars, two rows.
///
/// The vocabularies here are small and the names short, so the allocation per
/// call is not worth avoiding.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The (at most three) candidates nearest `typed`, closest first.
///
/// Empty when nothing is near enough, which is the answer for a string that was
/// not a typo of anything: three confident wrong guesses are worse than none.
/// Ties break by name so the list is stable run to run.
pub fn nearest<I, S>(typed: &str, candidates: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let typed_len = typed.chars().count();
    let mut scored: Vec<(usize, String)> = candidates
        .into_iter()
        .filter_map(|c| {
            let name = c.as_ref();
            let d = edit_distance(typed, name);
            (d <= max_edits(typed_len.max(name.chars().count()))).then(|| (d, name.to_string()))
        })
        .collect();
    scored.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
    scored.into_iter().take(3).map(|(_, n)| n).collect()
}

/// Render a suggestion list as the tail of an error message.
///
/// Empty string when there is nothing near, so the caller can append it
/// unconditionally without composing two message shapes.
#[must_use]
pub fn did_you_mean(candidates: &[String]) -> String {
    if candidates.is_empty() {
        String::new()
    } else {
        format!(". Did you mean {}?", candidates.join(", "))
    }
}

#[cfg(test)]
mod tests;
