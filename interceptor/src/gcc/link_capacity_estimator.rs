use super::data_rate::DataRate;

pub struct LinkCapacityEstimator {
    estimate_kbps: Option<f64>,
    deviation_kbps: f64,
}

impl LinkCapacityEstimator {
    pub fn new() -> Self {
        Self {
            estimate_kbps: None,
            deviation_kbps: 0.4,
        }
    }

    pub fn upper_bound(&self) -> DataRate {
        match self.estimate_kbps {
            Some(estimate_kbps) => DataRate::from_kilobits_per_sec(
                (estimate_kbps + 3.0 * self.deviation_estimate_kbps(estimate_kbps)) as i64,
            ),
            None => DataRate::infinity(),
        }
    }

    pub fn lower_bound(&self) -> DataRate {
        match self.estimate_kbps {
            Some(estimate_kbps) => {
                let kilobits_per_sec = f64::max(
                    0.0,
                    estimate_kbps - 3.0 * self.deviation_estimate_kbps(estimate_kbps),
                );
                DataRate::from_kilobits_per_sec(kilobits_per_sec as i64)
            }
            None => DataRate::zero(),
        }
    }

    pub fn reset(&mut self) {
        self.estimate_kbps = None;
    }

    pub fn on_overuse_detected(&mut self, acknowledged_rate: DataRate) {
        self.update(acknowledged_rate, 0.05);
    }

    pub fn on_probe_rate(&mut self, probe_rate: DataRate) {
        self.update(probe_rate, 0.5);
    }

    fn update(&mut self, capacity_sample: DataRate, alpha: f64) {
        let sample_kbps = capacity_sample.kbps() as f64;
        match self.estimate_kbps {
            Some(estimate_kbps) => {
                self.estimate_kbps = Some((1.0 - alpha) * estimate_kbps + alpha * sample_kbps);
            }
            None => {
                self.estimate_kbps = Some(sample_kbps);
            }
        }
        // Estimate the variance of the link capacity estimate and normalize the
        // variance with the link capacity estimate.
        let norm = f64::max(self.estimate_kbps.unwrap(), 1.0);
        let error_kbps = self.estimate_kbps.unwrap() - sample_kbps;
        self.deviation_kbps =
            (1.0 - alpha) * self.deviation_kbps + alpha * error_kbps * error_kbps / norm;
        // 0.4 ~= 14 kbit/s at 500 kbit/s
        // 2.5f ~= 35 kbit/s at 500 kbit/s
        self.deviation_kbps = f64::clamp(self.deviation_kbps, 0.4, 2.5);
    }

    pub fn has_estimate(&self) -> bool {
        self.estimate_kbps.is_some()
    }

    pub fn estimate(&self) -> DataRate {
        DataRate::from_kilobits_per_sec(self.estimate_kbps.unwrap() as i64)
    }

    fn deviation_estimate_kbps(&self, estimate_kbps: f64) -> f64 {
        // Calculate the max bit rate std dev given the normalized
        // variance and the current throughput bitrate. The standard deviation will
        // only be used if estimate_kbps_ has a value.
        f64::sqrt(self.deviation_kbps * estimate_kbps)
    }
}
