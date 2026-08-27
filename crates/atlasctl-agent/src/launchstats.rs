// SPDX-License-Identifier: AGPL-3.0-only

//! What a running model is doing, sampled from its own `/metrics`.
//!
//! Two scrapes are needed for anything rate-shaped, so this holds the previous
//! one. That state is the whole reason this is a type rather than a function:
//! throughput is a difference, and a difference needs somewhere to remember.
//!
//! **Absent is not zero.** Every field is optional, and stays `None` when the
//! engine does not report it or when there is not yet a second sample to
//! difference against. A dashboard that renders 0 tok/s for "not measured yet"
//! teaches an operator to distrust it, and the one time throughput really is
//! zero they will not believe the number.

use anyhow::Result;
use atlasctl_core::metrics::{self, Scrape};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fetches an exposition body from a served model.
///
/// A trait so the derivations above it can be tested without a model: the
/// arithmetic is where these things go wrong, not the socket.
pub trait MetricsSource: Send + Sync {
    /// Fetch `/metrics` from a model serving on this host.
    ///
    /// # Errors
    /// If the port is not answering, or does not answer in time — which is the
    /// ordinary state of a model that is still loading its weights.
    fn scrape(&self, port: u16) -> Result<String>;
}

/// One reading, ready to render.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaunchStats {
    /// Requests served since the engine started.
    pub requests_total: Option<f64>,
    /// Requests in flight right now.
    pub requests_active: Option<f64>,
    /// Generated tokens per second, over the interval between the last two
    /// samples.
    pub decode_tokens_per_s: Option<f64>,
    /// Prompt tokens per second, likewise.
    pub prompt_tokens_per_s: Option<f64>,
    /// Median time to first token, seconds, estimated from the histogram.
    pub ttft_p50_s: Option<f64>,
    /// 90th percentile time to first token, seconds.
    pub ttft_p90_s: Option<f64>,
    /// Share of drafted tokens accepted, 0..1, when the model speculates.
    pub accept_rate: Option<f64>,
    /// Share of prefix-cache lookups that hit, 0..1.
    pub prefix_hit_rate: Option<f64>,
    /// Mean prompt tokens per request completed in the window.
    ///
    /// `None` when no request completed: a mean over nothing is undefined, and
    /// 0 would assert that the traffic carried no prompt — a measurement nobody
    /// made. See `metrics::mean_per_request` for the bias this knowingly has.
    pub isl_mean: Option<f64>,
    /// Mean generated tokens per request completed in the window, likewise.
    pub osl_mean: Option<f64>,
    /// Seconds the rates were measured over, so the page can say how fresh
    /// they are rather than implying they are instantaneous.
    pub window_s: Option<f64>,
}

/// Samples one launch, remembering enough to compute rates.
pub struct LaunchSampler {
    source: Box<dyn MetricsSource>,
    last: Mutex<Option<(Instant, Scrape)>>,
}

impl std::fmt::Debug for LaunchSampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchSampler").finish_non_exhaustive()
    }
}

/// Beyond this, the previous sample is too old to difference against.
///
/// A rate computed across a gap — a laptop asleep, a tab in the background —
/// is an average over a period nobody was watching, presented as what is
/// happening now.
const MAX_WINDOW: Duration = Duration::from_secs(60);

impl LaunchSampler {
    /// Build a sampler.
    #[must_use]
    pub fn new(source: Box<dyn MetricsSource>) -> Self {
        Self {
            source,
            last: Mutex::new(None),
        }
    }

    /// Take a reading.
    ///
    /// `now` is passed in rather than read here so the rate arithmetic can be
    /// tested without sleeping.
    ///
    /// # Errors
    /// If the model is not answering — which is the ordinary state of one that
    /// is still loading.
    pub fn sample(&self, port: u16, now: Instant) -> Result<LaunchStats> {
        let body = self.source.scrape(port)?;
        let current = metrics::parse(&body);

        let mut out = LaunchStats {
            requests_total: current.get("atlas_requests_total"),
            requests_active: current.get("atlas_requests_active"),
            ttft_p50_s: current
                .buckets
                .get("atlas_time_to_first_token_seconds")
                .and_then(|b| metrics::quantile(b, 0.50)),
            ttft_p90_s: current
                .buckets
                .get("atlas_time_to_first_token_seconds")
                .and_then(|b| metrics::quantile(b, 0.90)),
            accept_rate: accept_rate(&current),
            prefix_hit_rate: current.get("atlas_prefix_cache_hit_rate"),
            ..LaunchStats::default()
        };

        let mut slot = self.last.lock().expect("sampler lock poisoned");
        if let Some((then, previous)) = slot.as_ref() {
            let elapsed = now.saturating_duration_since(*then);
            if elapsed <= MAX_WINDOW && elapsed > Duration::ZERO {
                let secs = elapsed.as_secs_f64();
                out.window_s = Some(secs);
                out.decode_tokens_per_s =
                    rate_of(previous, &current, "atlas_generation_tokens_total", secs);
                out.prompt_tokens_per_s =
                    rate_of(previous, &current, "atlas_prompt_tokens_total", secs);
                // Per-request means over the SAME interval as the rates, so the
                // window caption already on screen describes them too.
                out.isl_mean = mean_over(previous, &current, "atlas_prompt_tokens_total");
                out.osl_mean = mean_over(previous, &current, "atlas_generation_tokens_total");
            }
        }
        *slot = Some((now, current));
        Ok(out)
    }

    /// Forget the previous sample.
    ///
    /// Called when a launch stops, so a later launch on the same port cannot
    /// difference its counters against a dead engine's — which would read as a
    /// counter reset at best, and a wildly wrong rate at worst.
    pub fn reset(&self) {
        *self.last.lock().expect("sampler lock poisoned") = None;
    }
}

fn rate_of(previous: &Scrape, current: &Scrape, name: &str, secs: f64) -> Option<f64> {
    metrics::rate(previous.get(name)?, current.get(name)?, secs)
}

/// Mean of `name` per request completed between the two scrapes.
///
/// Needs both counters from both scrapes: an engine that stopped exporting one
/// of them mid-window yields nothing rather than a mean computed against a
/// missing half.
fn mean_over(previous: &Scrape, current: &Scrape, name: &str) -> Option<f64> {
    metrics::mean_per_request(
        previous.get(name)?,
        current.get(name)?,
        previous.get("atlas_requests_total")?,
        current.get("atlas_requests_total")?,
    )
}

/// Draft acceptance, read per label because the sum of accepts and rejects is
/// a number with no meaning.
fn accept_rate(s: &Scrape) -> Option<f64> {
    const METRIC: &str = "atlas_spec_decode_verify_total";
    let accepted = s.sum_where(METRIC, "outcome", "accept")?;
    let total = s.get(METRIC)?;
    metrics::accept_rate(accepted, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Serves scripted bodies in order.
    pub(super) struct Scripted(StdMutex<Vec<String>>);

    impl Scripted {
        pub(super) fn new(bodies: &[&str]) -> Box<Self> {
            Box::new(Self(StdMutex::new(
                bodies.iter().rev().map(|s| (*s).to_owned()).collect(),
            )))
        }
    }

    impl MetricsSource for Scripted {
        fn scrape(&self, _: u16) -> Result<String> {
            self.0
                .lock()
                .expect("lock")
                .pop()
                .ok_or_else(|| anyhow::anyhow!("not answering"))
        }
    }

    const REAL: &str = include_str!("../../atlasctl-core/src/metrics-sample.txt");

    fn body(gen_tokens: u32, prompt_tokens: u32) -> String {
        format!(
            "atlas_requests_total 4\n\
             atlas_requests_active 1\n\
             atlas_generation_tokens_total {gen_tokens}\n\
             atlas_prompt_tokens_total {prompt_tokens}\n"
        )
    }

    /// A dashboard that renders 0 tok/s for "not measured yet" teaches an
    /// operator to distrust it, and the one time throughput really is zero
    /// they will not believe the number.
    #[test]
    fn the_first_sample_has_no_rate_because_a_rate_needs_two() {
        let s = LaunchSampler::new(Scripted::new(&[&body(100, 10)]));
        let out = s.sample(8888, Instant::now()).expect("samples");
        assert_eq!(out.decode_tokens_per_s, None);
        assert_eq!(out.window_s, None);
        // Point-in-time values are available immediately, though.
        assert_eq!(out.requests_total, Some(4.0));
        assert_eq!(out.requests_active, Some(1.0));
    }

    #[test]
    fn the_second_sample_gives_a_rate_over_the_interval_it_covers() {
        let s = LaunchSampler::new(Scripted::new(&[&body(100, 10), &body(340, 30)]));
        let t0 = Instant::now();
        s.sample(8888, t0).expect("first");
        let out = s.sample(8888, t0 + Duration::from_secs(2)).expect("second");
        assert_eq!(out.decode_tokens_per_s, Some(120.0));
        assert_eq!(out.prompt_tokens_per_s, Some(10.0));
        assert_eq!(out.window_s, Some(2.0));
    }

    /// The engine restarting must not read as enormous negative throughput.
    #[test]
    fn a_restarted_engine_reports_no_rate_rather_than_a_negative_one() {
        let s = LaunchSampler::new(Scripted::new(&[&body(5000, 500), &body(12, 2)]));
        let t0 = Instant::now();
        s.sample(8888, t0).expect("first");
        let out = s.sample(8888, t0 + Duration::from_secs(1)).expect("second");
        assert_eq!(out.decode_tokens_per_s, None);
    }

    /// A rate across a gap — a sleeping laptop, a backgrounded tab — is an
    /// average over a period nobody watched, presented as what is happening
    /// now.
    #[test]
    fn a_stale_previous_sample_is_not_differenced_against() {
        let s = LaunchSampler::new(Scripted::new(&[&body(100, 10), &body(999_999, 9999)]));
        let t0 = Instant::now();
        s.sample(8888, t0).expect("first");
        let out = s
            .sample(8888, t0 + Duration::from_secs(600))
            .expect("second");
        assert_eq!(out.decode_tokens_per_s, None);
        assert_eq!(out.window_s, None);
    }

    /// A later launch on the same port must not difference its counters
    /// against a dead engine's.
    #[test]
    fn a_reset_forgets_the_previous_engine() {
        let s = LaunchSampler::new(Scripted::new(&[&body(100, 10), &body(340, 30)]));
        let t0 = Instant::now();
        s.sample(8888, t0).expect("first");
        s.reset();
        let out = s.sample(8888, t0 + Duration::from_secs(2)).expect("second");
        assert_eq!(out.decode_tokens_per_s, None);
    }

    /// A model still loading its weights is not answering, and that is an
    /// ordinary state rather than a failure to hide.
    #[test]
    fn a_model_that_is_not_answering_yet_is_an_error_the_caller_can_show() {
        let s = LaunchSampler::new(Scripted::new(&[]));
        assert!(s.sample(8888, Instant::now()).is_err());
    }

    #[test]
    fn a_real_scrape_yields_the_fields_the_dashboard_renders() {
        let s = LaunchSampler::new(Scripted::new(&[REAL]));
        let out = s.sample(8888, Instant::now()).expect("samples");
        assert_eq!(out.requests_total, Some(1.0));
        assert_eq!(out.requests_active, Some(0.0));
        assert!(out.ttft_p50_s.is_some_and(|v| v > 0.0), "{out:?}");
        // Every verify in that capture was an accept.
        assert_eq!(out.accept_rate, Some(1.0));
        assert_eq!(out.prefix_hit_rate, Some(0.0));
    }

    /// An engine that reports no speculation has no acceptance rate; 0% would
    /// read as a broken speculator rather than one that is switched off.
    #[test]
    fn a_model_that_does_not_speculate_has_no_acceptance_rate() {
        let s = LaunchSampler::new(Scripted::new(&[&body(1, 1)]));
        let out = s.sample(8888, Instant::now()).expect("samples");
        assert_eq!(out.accept_rate, None);
    }
}

#[cfg(test)]
mod isl_osl_tests {
    use super::*;

    /// Two scrapes an interval apart: 4 requests finished, carrying 2048 prompt
    /// tokens and 512 generated ones between them.
    #[test]
    fn the_means_are_per_request_over_the_same_window_as_the_rates() {
        let sampler = LaunchSampler::new(super::tests::Scripted::new(&[
            "atlas_requests_total 10\natlas_requests_active 0\n\
             atlas_generation_tokens_total 1000\natlas_prompt_tokens_total 5000\n",
            "atlas_requests_total 14\natlas_requests_active 0\n\
             atlas_generation_tokens_total 1512\natlas_prompt_tokens_total 7048\n",
        ]));
        let t0 = Instant::now();
        let first = sampler.sample(9000, t0).expect("first");
        // No previous sample: no window, so no rates and no means.
        assert_eq!(first.isl_mean, None, "a first poll measures no interval");
        assert_eq!(first.osl_mean, None);

        let second = sampler
            .sample(9000, t0 + Duration::from_secs(4))
            .expect("second");
        assert_eq!(
            second.isl_mean,
            Some(512.0),
            "2048 prompt tokens / 4 requests"
        );
        assert_eq!(second.osl_mean, Some(128.0), "512 generated / 4 requests");
        assert_eq!(second.window_s, Some(4.0));
    }

    /// Tokens moved but nothing finished. The mean is undefined, and zero would
    /// claim the window's traffic carried no tokens.
    #[test]
    fn tokens_without_a_completed_request_report_nothing_not_zero() {
        let sampler = LaunchSampler::new(super::tests::Scripted::new(&[
            "atlas_requests_total 10\natlas_generation_tokens_total 1000\n\
             atlas_prompt_tokens_total 5000\n",
            "atlas_requests_total 10\natlas_generation_tokens_total 1400\n\
             atlas_prompt_tokens_total 5600\n",
        ]));
        let t0 = Instant::now();
        sampler.sample(9000, t0).expect("first");
        let out = sampler
            .sample(9000, t0 + Duration::from_secs(4))
            .expect("second");
        assert_eq!(out.isl_mean, None);
        assert_eq!(out.osl_mean, None);
        // The rates still exist: tokens per second is defined over the window
        // whether or not a request happened to finish inside it.
        assert!(out.decode_tokens_per_s.is_some());
    }
}
