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

/// The mean of a per-request quantity over an interval: Δtotal ÷ Δrequests.
///
/// `None` rather than a number whenever the answer would be invented:
///
/// * no requests completed in the interval — a mean over nothing is undefined,
///   and reporting 0 would say "requests in this window carried no tokens",
///   which is a measurement nobody made;
/// * either counter went backwards, which is a restart and tells us nothing
///   about the interval it spans;
/// * the result is not finite.
///
/// The value is APPROXIMATE and knowingly so: tokens accrue while a request is
/// in flight, but `requests_total` only increments when it finishes, so a long
/// request's tokens land in an earlier window than its completion. Over a
/// steady stream the bias is small; over a handful of long requests it is not.
/// That is why this is a mean labelled with its window rather than a per-request
/// statistic, and why a percentile would need a histogram from the engine.
#[must_use]
pub fn mean_per_request(
    prev_total: f64,
    cur_total: f64,
    prev_requests: f64,
    cur_requests: f64,
) -> Option<f64> {
    if cur_total < prev_total || cur_requests < prev_requests {
        return None;
    }
    let requests = cur_requests - prev_requests;
    if requests <= 0.0 {
        return None;
    }
    let mean = (cur_total - prev_total) / requests;
    mean.is_finite().then_some(mean)
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
#[path = "metrics/tests.rs"]
mod tests;

#[cfg(test)]
mod mean_tests {
    use super::mean_per_request;

    #[test]
    fn a_mean_is_tokens_over_requests_in_the_same_interval() {
        // 4 requests carried 2048 prompt tokens between them.
        assert_eq!(mean_per_request(1000.0, 3048.0, 10.0, 14.0), Some(512.0));
    }

    /// The case that must never read as zero. No request finished, so nothing
    /// was measured — and "0 tokens per request" is a claim about traffic that
    /// did not happen.
    #[test]
    fn no_completed_requests_is_unknown_not_zero() {
        assert_eq!(mean_per_request(1000.0, 1500.0, 10.0, 10.0), None);
    }

    /// A restart resets both counters. The interval spans two different engine
    /// lifetimes and describes neither.
    #[test]
    fn a_counter_that_went_backwards_yields_nothing() {
        assert_eq!(mean_per_request(1000.0, 5.0, 10.0, 14.0), None);
        assert_eq!(mean_per_request(1000.0, 3048.0, 10.0, 2.0), None);
    }

    #[test]
    fn a_non_finite_result_is_refused() {
        assert_eq!(mean_per_request(0.0, f64::INFINITY, 0.0, 1.0), None);
        assert_eq!(mean_per_request(0.0, f64::NAN, 0.0, 1.0), None);
    }
}
