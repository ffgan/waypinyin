//! Opt-in bounded, content-free timing of the engine's key handler.
use std::time::Duration;

const WINDOW: usize = 256;

#[derive(Clone, Copy)]
struct Sample {
    handle: u64,
    present: u64,
    total: u64,
}

pub(crate) struct Latency {
    samples: Vec<Sample>,
}

impl Latency {
    pub(crate) fn from_env() -> Option<Self> {
        (std::env::var_os("WAYPINYIN_TRACE_LATENCY").as_deref() == Some(std::ffi::OsStr::new("1")))
            .then(|| Self {
                samples: Vec::with_capacity(WINDOW),
            })
    }

    pub(crate) fn record(&mut self, handle: Duration, total: Duration) {
        self.samples.push(Sample {
            handle: nanos(handle),
            present: nanos(total.saturating_sub(handle)),
            total: nanos(total),
        });
        if self.samples.len() == WINDOW {
            self.report();
        }
    }

    fn report(&mut self) {
        if self.samples.is_empty() {
            return;
        }
        for (stage, field) in [
            ("handle", (|s: &Sample| s.handle) as fn(&Sample) -> u64),
            ("present", (|s: &Sample| s.present) as fn(&Sample) -> u64),
            ("total", (|s: &Sample| s.total) as fn(&Sample) -> u64),
        ] {
            let mut values: Vec<_> = self.samples.iter().map(field).collect();
            values.sort_unstable();
            eprintln!(
                "waypinyin: latency stage={stage} count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
                values.len(),
                percentile(&values, 50),
                percentile(&values, 95),
                percentile(&values, 99),
                values.last().unwrap(),
            );
        }
        self.samples.clear();
    }
}

impl Drop for Latency {
    fn drop(&mut self) {
        self.report();
    }
}

fn nanos(value: Duration) -> u64 {
    value.as_nanos().min(u64::MAX as u128) as u64
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_windows_and_nearest_rank_percentiles() {
        let mut latency = Latency {
            samples: Vec::with_capacity(WINDOW),
        };
        for _ in 0..WINDOW - 1 {
            latency.record(Duration::from_nanos(3), Duration::from_nanos(5));
        }
        assert_eq!(latency.samples.len(), WINDOW - 1);
        assert_eq!(latency.samples[0].present, 2);
        latency.record(Duration::from_nanos(3), Duration::from_nanos(5));
        assert!(latency.samples.is_empty());
        assert_eq!(percentile(&[1], 99), 1);
        let values: Vec<_> = (1..=100).collect();
        assert_eq!(percentile(&values, 50), 50);
        assert_eq!(percentile(&values, 95), 95);
        assert_eq!(percentile(&values, 99), 99);
    }
}
