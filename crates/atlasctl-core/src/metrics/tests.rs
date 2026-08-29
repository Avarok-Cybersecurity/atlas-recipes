// SPDX-License-Identifier: AGPL-3.0-only

//! Parser tests for [`super`], split out for headroom: the file sat at 493 of a
//! 500-line cap, and a cap crossed by accumulation fails on whichever pull
//! request is open next rather than the one responsible.

use super::*;

/// A real scrape from `avarok/atlas-gb10` serving Qwen3.6-35B-A3B-FP8 on a
/// GB10, captured after one completion (2026-08-26). Tested against the
/// real thing rather than an invented sample, because the shapes that break
/// a parser — labelled counters, a `model=` label on every histogram
/// bucket, four-decimal gauges — are exactly what an invented one omits.
const REAL: &str = include_str!("../metrics-sample.txt");

#[test]
fn a_real_scrape_yields_the_counters_an_operator_watches() {
    let s = parse(REAL);
    assert_eq!(s.get("atlas_requests_total"), Some(1.0));
    assert_eq!(s.get("atlas_generation_tokens_total"), Some(120.0));
    assert_eq!(s.get("atlas_prompt_tokens_total"), Some(19.0));
    assert_eq!(s.get("atlas_requests_active"), Some(0.0));
}

#[test]
fn comments_and_type_lines_are_not_samples() {
    let s = parse(REAL);
    assert!(!s.values.contains_key("#"));
    assert!(!s.values.contains_key("HELP"));
}

/// The histogram carries a `model=` label on every bucket, so a parser that
/// keyed on the raw line would find no histogram at all.
#[test]
fn a_labelled_histogram_is_still_a_histogram() {
    let s = parse(REAL);
    let b = s
        .buckets
        .get("atlas_time_to_first_token_seconds")
        .expect("the TTFT histogram");
    assert!(b.len() > 3, "buckets: {b:?}");
    assert!(
        b.windows(2).all(|w| w[0].0 <= w[1].0),
        "buckets must be sorted by upper bound"
    );
    assert!(
        b.windows(2).all(|w| w[0].1 <= w[1].1),
        "a cumulative histogram never decreases"
    );
    let (sum, count) = s.sums["atlas_time_to_first_token_seconds"];
    assert_eq!(count, 1.0);
    assert!(sum > 0.0);
}

/// Summing across `outcome` would add accepts to rejects and produce a
/// number with no meaning. This is the metric that forced label fidelity.
#[test]
fn a_partitioned_counter_is_read_per_label() {
    let s = parse(REAL);
    let accepted = s
        .sum_where("atlas_spec_decode_verify_total", "outcome", "accept")
        .expect("the verify counter");
    assert_eq!(accepted, 5.0);
    // Nothing was rejected in this capture, and that is a real zero rather
    // than a missing metric.
    assert_eq!(
        s.sum_where("atlas_spec_decode_verify_total", "outcome", "reject"),
        Some(0.0)
    );
    assert_eq!(s.sum_where("atlas_nonexistent", "outcome", "accept"), None);
}

#[test]
fn an_absent_metric_is_absent_rather_than_zero() {
    let s = parse(REAL);
    assert_eq!(s.get("atlas_kv_cache_utilization"), None);
}

#[test]
fn a_value_with_no_labels_parses() {
    let s = parse("plain_metric 42\n");
    assert_eq!(s.get("plain_metric"), Some(42.0));
}

/// A trailing timestamp is legal in the exposition format.
#[test]
fn a_trailing_timestamp_is_not_mistaken_for_the_value() {
    let s = parse("m 7 1699999999000\n");
    assert_eq!(s.get("m"), Some(7.0));
}

#[test]
fn nan_and_inf_samples_are_skipped_rather_than_poisoning_a_total() {
    let s = parse("m{a=\"1\"} NaN\nm{a=\"2\"} 3\n");
    assert_eq!(s.get("m"), Some(3.0));
}

mod rates {
    use super::super::*;

    #[test]
    fn a_counter_delta_over_an_interval_is_a_rate() {
        assert_eq!(rate(100.0, 340.0, 2.0), Some(120.0));
    }

    /// The engine restarting must not read as a large negative throughput.
    /// Showing nothing is better than showing a number that is wrong.
    #[test]
    fn a_counter_that_went_backwards_is_a_restart_not_a_negative_rate() {
        assert_eq!(rate(5000.0, 12.0, 1.0), None);
    }

    #[test]
    fn a_zero_or_nonsense_interval_yields_nothing() {
        assert_eq!(rate(0.0, 10.0, 0.0), None);
        assert_eq!(rate(0.0, 10.0, -1.0), None);
        assert_eq!(rate(0.0, 10.0, f64::NAN), None);
    }

    #[test]
    fn an_unchanged_counter_is_a_real_zero() {
        assert_eq!(rate(10.0, 10.0, 5.0), Some(0.0));
    }
}

mod quantiles {
    use super::super::*;
    use super::REAL;

    fn h() -> Vec<(f64, f64)> {
        vec![(0.1, 0.0), (0.5, 2.0), (1.0, 6.0), (f64::INFINITY, 6.0)]
    }

    #[test]
    fn a_quantile_interpolates_inside_its_bucket() {
        // p50 of 6 observations wants the 3rd; it sits in (0.5, 1.0] one
        // quarter of the way through that bucket's 4 observations.
        let p50 = quantile(&h(), 0.5).expect("p50");
        assert!((p50 - 0.625).abs() < 1e-9, "{p50}");
    }

    /// An unbounded bucket has no upper edge to interpolate toward, and
    /// inventing one would report a latency the model never had.
    #[test]
    fn a_quantile_falling_in_the_inf_bucket_is_not_reported() {
        let tail = vec![(1.0, 1.0), (f64::INFINITY, 10.0)];
        assert_eq!(quantile(&tail, 0.9), None);
    }

    #[test]
    fn an_empty_or_unobserved_histogram_yields_nothing() {
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&[(1.0, 0.0), (f64::INFINITY, 0.0)], 0.5), None);
    }

    #[test]
    fn a_quantile_outside_zero_to_one_is_refused() {
        assert_eq!(quantile(&h(), 1.5), None);
        assert_eq!(quantile(&h(), -0.1), None);
    }

    #[test]
    fn the_real_histogram_gives_a_ttft_in_a_plausible_range() {
        let s = parse(REAL);
        let b = &s.buckets["atlas_time_to_first_token_seconds"];
        let p50 = quantile(b, 0.5).expect("one observation is enough for p50");
        assert!(p50 > 0.0 && p50 < 60.0, "TTFT p50 was {p50}s");
    }
}

mod acceptance {
    use super::super::*;

    #[test]
    fn acceptance_is_a_share_of_what_was_drafted() {
        assert_eq!(accept_rate(3.0, 4.0), Some(0.75));
    }

    /// A model that has not been asked anything has no acceptance rate, and
    /// 0% would read as a broken speculator rather than an idle one.
    #[test]
    fn nothing_drafted_has_no_rate_rather_than_a_rate_of_zero() {
        assert_eq!(accept_rate(0.0, 0.0), None);
    }

    #[test]
    fn a_rate_cannot_exceed_one_however_the_counters_disagree() {
        assert_eq!(accept_rate(9.0, 4.0), Some(1.0));
    }
}
