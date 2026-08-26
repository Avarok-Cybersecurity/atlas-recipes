// SPDX-License-Identifier: AGPL-3.0-only

//! Reading a served model's own `/metrics`, and deriving the numbers an
//! operator actually watches.
//!
//! **Scraped, never parsed out of logs.** A log line is prose that changes
//! whenever someone rewords it; an exposition endpoint is a contract versioned
//! with the engine. The tool this replaces read throughput out of log text, and
//! every reword silently broke it.
//!
//! Everything here is pure: text in, numbers out. The I/O that fetches the text
//! lives elsewhere, so every derivation below — and in particular the counter
//! arithmetic, which is where these things go wrong — is testable without a
//! model, a GPU or a socket.

use std::collections::BTreeMap;

/// One labelled sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    /// Label set, as written.
    pub labels: BTreeMap<String, String>,
    /// The value.
    pub value: f64,
}

/// One scrape, reduced to the series we use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scrape {
    /// Plain counters and gauges, by metric name, summed across labels.
    ///
    /// Convenient, and wrong for anything whose labels *partition* the meaning.
    /// `atlas_spec_decode_verify_total` is the live example: it is labelled by
    /// `outcome`, so the sum of accepts and rejects is a number with no meaning
    /// at all. Use [`Scrape::sum_where`] for those.
    pub values: BTreeMap<String, f64>,
    /// Every sample with its labels intact, by metric name.
    pub series: BTreeMap<String, Vec<Series>>,
    /// Histogram buckets, by metric name: (upper bound, cumulative count).
    pub buckets: BTreeMap<String, Vec<(f64, f64)>>,
    /// Histogram sums and counts, by metric name.
    pub sums: BTreeMap<String, (f64, f64)>,
}

impl Scrape {
    /// Read one metric, summed across its labels.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f64> {
        self.values.get(name).copied()
    }

    /// Sum only the samples whose `label` equals `value`.
    ///
    /// `None` when the metric is absent entirely, so a caller can tell "the
    /// engine does not report this" from "it reports zero" — the distinction
    /// the whole telemetry surface is built on.
    #[must_use]
    pub fn sum_where(&self, name: &str, label: &str, value: &str) -> Option<f64> {
        let series = self.series.get(name)?;
        Some(
            series
                .iter()
                .filter(|s| s.labels.get(label).is_some_and(|v| v == value))
                .map(|s| s.value)
                .sum(),
        )
    }
}

/// Parse Prometheus text exposition.
///
/// Labels are folded away by summing across them: this surface shows one
/// number per metric, and a per-label breakdown nobody renders would only be a
/// place for the totals to disagree with themselves.
#[must_use]
pub fn parse(text: &str) -> Scrape {
    let mut out = Scrape::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((head, tail)) = split_value(line) else {
            continue;
        };
        let Ok(value) = tail.parse::<f64>() else {
            continue;
        };
        // `NaN` and `+Inf` are legal in the exposition format, and Rust parses
        // both happily. One of either poisons every total it is added to, for
        // good — a gauge that reads NaN once reads NaN until the process
        // restarts. Dropped here so a single bad sample cannot take a whole
        // dashboard with it.
        if !value.is_finite() {
            continue;
        }
        let (name, labels) = split_labels(head);

        if let Some(base) = name.strip_suffix("_bucket") {
            if let Some(le) = labels.get("le").and_then(|v| parse_le(v)) {
                out.buckets
                    .entry(base.to_owned())
                    .or_default()
                    .push((le, value));
            }
        } else if let Some(base) = name.strip_suffix("_sum") {
            out.sums.entry(base.to_owned()).or_default().0 += value;
        } else if let Some(base) = name.strip_suffix("_count") {
            out.sums.entry(base.to_owned()).or_default().1 += value;
        } else {
            *out.values.entry(name.to_owned()).or_insert(0.0) += value;
            out.series.entry(name.to_owned()).or_default().push(Series {
                labels: labels
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                value,
            });
        }
    }
    for b in out.buckets.values_mut() {
        b.sort_by(|a, c| a.0.total_cmp(&c.0));
    }
    out
}

/// Split a line into everything before the value and the value itself.
///
/// Splitting on the *last* whitespace rather than the first: a label value may
/// contain spaces, and splitting at the front would cut the metric in half.
fn split_value(line: &str) -> Option<(&str, &str)> {
    let (head, tail) = line.rsplit_once(char::is_whitespace)?;
    // A trailing timestamp is optional in the format. When present the value is
    // the field before it.
    if tail.parse::<f64>().is_err() {
        return None;
    }
    match head.trim_end().rsplit_once(char::is_whitespace) {
        Some((h2, maybe_value))
            if maybe_value.parse::<f64>().is_ok() && h2.contains(|c: char| !c.is_whitespace()) =>
        {
            Some((h2.trim_end(), maybe_value))
        }
        _ => Some((head.trim_end(), tail)),
    }
}

/// Split `name{a="b",c="d"}` into its name and labels.
fn split_labels(head: &str) -> (&str, BTreeMap<&str, &str>) {
    let Some(open) = head.find('{') else {
        return (head, BTreeMap::new());
    };
    let name = &head[..open];
    let body = head[open + 1..].trim_end_matches('}');
    let mut labels = BTreeMap::new();
    for pair in body.split(',') {
        if let Some((k, v)) = pair.split_once('=') {
            labels.insert(k.trim(), v.trim().trim_matches('"'));
        }
    }
    (name, labels)
}

fn parse_le(raw: &str) -> Option<f64> {
    match raw {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        other => other.parse().ok(),
    }
}

/// A rate derived from two scrapes of the same counter.
///
/// Returns `None` when there is no usable interval, and treats a counter that
/// went **backwards** as a restart rather than a negative rate: the engine
/// restarting would otherwise show up as a large negative throughput, which is
/// worse than showing nothing.
#[must_use]
pub fn rate(previous: f64, current: f64, seconds: f64) -> Option<f64> {
    if seconds <= 0.0 || !seconds.is_finite() {
        return None;
    }
    if current < previous {
        // A reset tells us nothing about the interval it spans.
        return None;
    }
    Some((current - previous) / seconds)
}

/// Estimate a quantile from cumulative histogram buckets.
///
/// Linear interpolation within the containing bucket, which is what Prometheus
/// itself does. Returns `None` rather than a number when the histogram is empty
/// or the quantile falls in the `+Inf` bucket — an unbounded bucket has no
/// upper edge to interpolate toward, and inventing one would report a latency
/// the model never had.
#[must_use]
pub fn quantile(buckets: &[(f64, f64)], q: f64) -> Option<f64> {
    if !(0.0..=1.0).contains(&q) || buckets.is_empty() {
        return None;
    }
    let total = buckets.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let want = q * total;
    let mut lower_bound = 0.0;
    let mut lower_count = 0.0;
    for &(edge, count) in buckets {
        if count >= want {
            if edge.is_infinite() {
                return None;
            }
            let span = count - lower_count;
            if span <= 0.0 {
                return Some(edge);
            }
            let frac = (want - lower_count) / span;
            return Some(lower_bound + (edge - lower_bound) * frac);
        }
        lower_bound = edge;
        lower_count = count;
    }
    None
}

/// The share of drafted tokens the model accepted, when it is speculating.
///
/// `None` rather than zero when nothing has been drafted: a model that has not
/// been asked anything has no acceptance rate, and rendering 0% would read as a
/// broken speculator rather than an idle one.
#[must_use]
pub fn accept_rate(accepted: f64, drafted: f64) -> Option<f64> {
    if drafted <= 0.0 {
        return None;
    }
    Some((accepted / drafted).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real scrape from `avarok/atlas-gb10` serving Qwen3.6-35B-A3B-FP8 on a
    /// GB10, captured after one completion (2026-08-26). Tested against the
    /// real thing rather than an invented sample, because the shapes that break
    /// a parser — labelled counters, a `model=` label on every histogram
    /// bucket, four-decimal gauges — are exactly what an invented one omits.
    const REAL: &str = include_str!("metrics-sample.txt");

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
}
