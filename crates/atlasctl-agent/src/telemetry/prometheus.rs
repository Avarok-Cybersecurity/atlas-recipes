// SPDX-License-Identifier: AGPL-3.0-only

//! Reading the model server's own metrics.
//!
//! Scraping the engine's `/metrics` is deliberately preferred over parsing its
//! logs: the endpoint is a versioned contract that ships with the engine, and a
//! log line is not. Only the handful of series we name are read; everything
//! else in the exposition is ignored.

use std::collections::BTreeMap;

/// A parsed exposition: series name (with labels dropped) to summed value.
///
/// Labels are summed rather than kept because every series we care about is
/// either unlabelled or wants its total. The one exception, draft acceptance,
/// needs two specific labels and reads them itself.
pub type Series = BTreeMap<String, f64>;

/// Parse Prometheus text exposition.
///
/// Tolerant by design: an unknown line, a malformed value, or a series we do
/// not recognise is skipped rather than failing the sample. A dashboard losing
/// one field is better than losing the whole reading.
pub fn parse(body: &str) -> Series {
    let mut out = Series::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((lhs, rhs)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(value) = rhs.trim().parse::<f64>() else {
            continue;
        };
        out.entry(lhs.trim().to_string())
            .and_modify(|v| *v += value)
            .or_insert(value);
        // Also accumulate under the bare name, so a caller can ask for a total
        // without knowing the label set.
        if let Some((name, _labels)) = lhs.split_once('{') {
            out.entry(name.trim().to_string())
                .and_modify(|v| *v += value)
                .or_insert(value);
        }
    }
    out
}

/// Read one series by bare name.
pub fn value(series: &Series, name: &str) -> Option<f64> {
    series.get(name).copied()
}

/// Estimate a histogram quantile from its buckets.
///
/// Linear interpolation within the bucket that crosses the target rank, which
/// is what Prometheus itself does. The result is an estimate and callers label
/// it as one — bucket boundaries bound how precise it can be.
pub fn histogram_quantile(series: &Series, metric: &str, q: f64) -> Option<f64> {
    let prefix = format!("{metric}_bucket{{le=\"");
    let mut buckets: Vec<(f64, f64)> = series
        .iter()
        .filter_map(|(k, v)| {
            let rest = k.strip_prefix(&prefix)?;
            let le = rest.split('"').next()?;
            let bound = if le == "+Inf" {
                f64::INFINITY
            } else {
                le.parse().ok()?
            };
            Some((bound, *v))
        })
        .collect();
    if buckets.is_empty() {
        return None;
    }
    buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let total = buckets.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let target = q * total;
    let mut prev_bound = 0.0;
    let mut prev_count = 0.0;
    for (bound, count) in buckets {
        if count >= target {
            if bound.is_infinite() {
                return Some(prev_bound);
            }
            let span = count - prev_count;
            let frac = if span > 0.0 {
                (target - prev_count) / span
            } else {
                0.0
            };
            return Some(prev_bound + (bound - prev_bound) * frac);
        }
        prev_bound = bound;
        prev_count = count;
    }
    Some(prev_bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like the engine's real exposition.
    const BODY: &str = r#"
# HELP atlas_requests_total Total requests processed
# TYPE atlas_requests_total counter
atlas_requests_total 1042
# TYPE atlas_requests_active gauge
atlas_requests_active 3
atlas_generation_tokens_total 88231
atlas_time_to_first_token_seconds_bucket{le="0.5"} 10
atlas_time_to_first_token_seconds_bucket{le="1"} 60
atlas_time_to_first_token_seconds_bucket{le="2.5"} 90
atlas_time_to_first_token_seconds_bucket{le="+Inf"} 100
atlas_spec_decode_verify_total{outcome="accepted"} 700
atlas_spec_decode_verify_total{outcome="rejected"} 300
"#;

    #[test]
    fn plain_series_are_read() {
        let s = parse(BODY);
        assert_eq!(value(&s, "atlas_requests_total"), Some(1042.0));
        assert_eq!(value(&s, "atlas_requests_active"), Some(3.0));
        assert_eq!(value(&s, "atlas_generation_tokens_total"), Some(88231.0));
    }

    #[test]
    fn labelled_series_are_summed_under_their_bare_name() {
        let s = parse(BODY);
        assert_eq!(value(&s, "atlas_spec_decode_verify_total"), Some(1000.0));
        // And remain addressable individually, for a rate that needs one label.
        assert_eq!(
            value(&s, r#"atlas_spec_decode_verify_total{outcome="accepted"}"#),
            Some(700.0)
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        assert!(!parse(BODY).keys().any(|k| k.starts_with('#')));
    }

    #[test]
    fn a_quantile_falls_inside_the_crossing_bucket() {
        let s = parse(BODY);
        let p50 = histogram_quantile(&s, "atlas_time_to_first_token_seconds", 0.5).unwrap();
        // The 50th of 100 observations lands in the (0.5, 1] bucket.
        assert!((0.5..=1.0).contains(&p50), "p50 was {p50}");
    }

    #[test]
    fn a_quantile_of_an_absent_histogram_is_absent_not_zero() {
        assert_eq!(histogram_quantile(&parse(BODY), "nope_seconds", 0.5), None);
    }

    #[test]
    fn malformed_input_is_skipped_rather_than_failing_the_sample() {
        let s = parse("good 1\nbad_no_value\nalso bad_value\n# comment\n");
        assert_eq!(value(&s, "good"), Some(1.0));
        assert_eq!(value(&s, "also"), None);
        assert!(
            !s.is_empty(),
            "one bad line must not lose the whole reading"
        );
    }

    #[test]
    fn an_empty_body_yields_nothing() {
        assert!(parse("").is_empty());
    }
}
