// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

#[test]
fn edit_distance_is_symmetric_and_counts_single_edits() {
    assert_eq!(edit_distance("", ""), 0);
    assert_eq!(edit_distance("abc", ""), 3);
    assert_eq!(edit_distance("", "abc"), 3);
    assert_eq!(edit_distance("abc", "abc"), 0);
    assert_eq!(edit_distance("unsloh", "unsloth"), 1);
    assert_eq!(edit_distance("unsloth", "unsloh"), 1);
    assert_eq!(edit_distance("gemma4", "gemma-4"), 1);
    assert_eq!(edit_distance("abc", "abd"), 1);
    // Multi-byte input must not panic, and must count chars not bytes.
    assert_eq!(edit_distance("héllo", "hello"), 1);
}

#[test]
fn the_edit_ceiling_stays_inside_its_stated_bounds() {
    assert_eq!(max_edits(0), 2, "floor");
    assert_eq!(max_edits(7), 2, "floor still applies to short names");
    assert_eq!(max_edits(12), 3);
    assert_eq!(max_edits(20), 5);
    assert_eq!(max_edits(200), 5, "cap");
}

/// The case the prefix-and-alphabetical rule got wrong: the right answer is
/// one character away but sorts after two others sharing the prefix.
#[test]
fn the_nearest_name_comes_first_not_the_alphabetically_first() {
    let vocab = [
        "qwen3.5-0.8b-bf16-atlas",
        "qwen3.5-122b-a10b-nvfp4-ep2",
        "qwen3.8-27b-nvfp4-unsloth",
    ];
    let got = nearest("qwen3.8-27b-nvfp4-unsloh", vocab);
    assert_eq!(
        got.first().map(String::as_str),
        Some("qwen3.8-27b-nvfp4-unsloth"),
        "{got:?}"
    );
}

#[test]
fn a_typo_inside_the_first_characters_is_still_found() {
    // Shares no six-character prefix with the answer, which is what the old
    // rule keyed on.
    assert_eq!(
        nearest("gemma4-31b-nvfp4", ["gemma-4-31b-nvfp4"]),
        ["gemma-4-31b-nvfp4"]
    );
    // NOT this one, and the fact is worth pinning: `max_seq_len` is what a
    // reader of the rendered command types, and `max_model_len` is the key --
    // but they are five edits apart, past the ceiling. Distance is the wrong
    // instrument for a rename; `flags::key_for_flag_spelling` answers it from
    // the table that holds both spellings.
    assert!(
        nearest("max_seq_len", ["max_model_len"]).is_empty(),
        "a rename is not a typo and must not be papered over by a loose ceiling"
    );
}

#[test]
fn an_unrelated_string_suggests_nothing_rather_than_guessing() {
    assert!(
        nearest(
            "nope/nothere",
            ["qwen3.8-27b-nvfp4-unsloth", "gemma-4-31b-nvfp4"]
        )
        .is_empty()
    );
    assert!(nearest("zzzzzzzzzzzzzzzzzzzz", ["port", "max_model_len"]).is_empty());
}

#[test]
fn at_most_three_are_offered_and_ties_are_stable() {
    let vocab = ["aaa1", "aaa2", "aaa3", "aaa4", "aaa5"];
    let got = nearest("aaa", vocab);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got, ["aaa1", "aaa2", "aaa3"], "ties break by name: {got:?}");
    assert_eq!(nearest("aaa", vocab), got, "same answer twice");
}

#[test]
fn the_message_tail_is_empty_when_there_is_nothing_to_suggest() {
    assert_eq!(did_you_mean(&[]), "");
    assert_eq!(did_you_mean(&["a".to_string()]), ". Did you mean a?");
    assert_eq!(
        did_you_mean(&["a".to_string(), "b".to_string()]),
        ". Did you mean a, b?"
    );
}

/// A name the operator did not finish typing must be offered.
///
/// This is the case edit distance cannot reach: `qwen3.8-flash-next` is SIX
/// edits from `qwen3.8-flash-next-nvfp4`, past any threshold that is not also
/// offering nonsense — and it is the obvious thing to type, because it is what
/// the model is called. Before this, that typed exactly right returned nothing
/// at all.
#[test]
fn an_unfinished_name_finds_the_recipe_it_is_the_front_of() {
    let got = super::nearest(
        "qwen3.8-flash-next",
        ["qwen3.8-flash-next-nvfp4", "qwen3.6-27b-fp8"],
    );
    assert_eq!(got, vec!["qwen3.8-flash-next-nvfp4".to_string()]);
}

/// Prefix matches come FIRST, because a name half-typed is a stronger signal
/// than a name mistyped.
#[test]
fn a_prefix_outranks_a_mere_typo() {
    let got = super::nearest("qwen3.6-27b", ["qwen3.6-27b-fp8", "qwen3.6-27c"]);
    assert_eq!(
        got.first().map(String::as_str),
        Some("qwen3.6-27b-fp8"),
        "{got:?}"
    );
}

/// Two characters must not confidently offer three recipes.
///
/// The guard is the whole reason this is length-gated: without it every short
/// string becomes a prefix of something, and the suggestion stops meaning
/// anything.
#[test]
fn a_couple_of_characters_is_not_treated_as_a_name() {
    let got = super::nearest("qw", ["qwen3.8-flash-next-nvfp4", "qwen3.6-27b-fp8"]);
    assert!(
        !got.contains(&"qwen3.8-flash-next-nvfp4".to_string()),
        "a two-character stub must not claim a 24-character name: {got:?}"
    );
}

/// An exact name is not "a prefix of itself" — it resolves, and never reaches
/// this path. Pinned so the `name.len() > typed.len()` guard cannot be dropped
/// as redundant.
#[test]
fn an_exact_name_is_not_suggested_against_itself() {
    let got = super::nearest("qwen3.6-27b-fp8", ["qwen3.6-27b-fp8"]);
    assert_eq!(got, vec!["qwen3.6-27b-fp8".to_string()], "{got:?}");
}
