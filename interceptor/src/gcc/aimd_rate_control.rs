use super::{
    data_rate::{DataRate, DataSize},
    link_capacity_estimator::LinkCapacityEstimator,
    network_types::NetworkStateEstimate,
    time::{TimeDelta, Timestamp},
};

const CONGESTION_CONTROLLER_MIN_BITRATE: DataRate = DataRate::from_bits_per_sec(5_000);
const DEFAULT_RTT: TimeDelta = TimeDelta::from_millis(200);
const DEFAULT_BACKOFF_FACTOR: f64 = 0.85;
const BITRATE_WINDOW: TimeDelta = TimeDelta::from_seconds(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RateControlState {
    Hold,
    Increase,
    Decrease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BandwidthUsage {
    Normal = 0,
    Underusing = 1,
    Overusing = 2,
}

pub struct RateControlInput {
    bw_state: BandwidthUsage,
    estimated_throughput: Option<DataRate>,
}

impl RateControlInput {
    pub fn new(bw_state: BandwidthUsage, estimated_throughput: Option<DataRate>) -> Self {
        Self {
            bw_state,
            estimated_throughput,
        }
    }
}

pub struct AimdRateControl {
    min_configured_bitrate: DataRate,
    max_configured_bitrate: DataRate,
    current_bitrate: DataRate,
    latest_estimated_throughput: DataRate,
    link_capacity: LinkCapacityEstimator,
    network_estimate: Option<NetworkStateEstimate>,
    rate_control_state: RateControlState,
    time_last_bitrate_change: Timestamp,
    time_last_bitrate_decrease: Timestamp,
    time_first_throughput_estimate: Timestamp,
    bitrate_is_initialized: bool,
    beta: f64,
    in_alr: bool,
    rtt: TimeDelta,
    send_side: bool,
    last_decrease: Option<DataRate>,
    no_bitrate_increase_in_alr: bool,
    subtract_additional_backoff_term: bool,
    disable_estimate_bounded_increase: bool,
    use_current_estimate_as_min_upper_bound: bool,
}

impl AimdRateControl {
    pub fn new(config: AimdRateControlConfig, send_side: bool) -> Self {
        let max_configured_bitrate = DataRate::from_kilobits_per_sec(30_000);
        Self {
            min_configured_bitrate: CONGESTION_CONTROLLER_MIN_BITRATE,
            max_configured_bitrate,
            current_bitrate: max_configured_bitrate,
            latest_estimated_throughput: max_configured_bitrate,
            link_capacity: LinkCapacityEstimator::new(),
            network_estimate: None,
            rate_control_state: RateControlState::Hold,
            time_last_bitrate_change: Timestamp::minus_infinity(),
            time_last_bitrate_decrease: Timestamp::minus_infinity(),
            time_first_throughput_estimate: Timestamp::minus_infinity(),
            bitrate_is_initialized: false,
            beta: config.beta,
            in_alr: false,
            rtt: DEFAULT_RTT,
            send_side,
            no_bitrate_increase_in_alr: config.no_bitrate_increase_in_alr,
            subtract_additional_backoff_term: config.subtract_additional_backoff_term,
            last_decrease: None,
            disable_estimate_bounded_increase: config.disable_estimate_bounded_increase,
            use_current_estimate_as_min_upper_bound: config.use_current_estimate_as_min_upper_bound,
        }
    }

    pub fn set_start_bitrate(&mut self, start_bitrate: DataRate) {
        self.current_bitrate = start_bitrate;
        self.latest_estimated_throughput = self.current_bitrate;
        self.bitrate_is_initialized = true;
    }

    pub fn set_min_bitrate(&mut self, min_bitrate: DataRate) {
        self.min_configured_bitrate = min_bitrate;
        self.current_bitrate = Ord::min(min_bitrate, self.current_bitrate);
    }

    pub fn valid_estimate(&self) -> bool {
        self.bitrate_is_initialized
    }

    pub fn get_feedback_interval(&self) -> TimeDelta {
        // Estimate how often we can send RTCP if we allocate up to 5% of bandwidth
        // to feedback.
        const RTCP_SIZE: DataSize = DataSize::from_bytes(80);
        let rtcp_bitrate = self.current_bitrate * 0.05;
        let interval: TimeDelta = RTCP_SIZE / rtcp_bitrate;
        interval.clamp(TimeDelta::from_millis(200), TimeDelta::from_millis(1000))
    }

    pub fn time_to_reduce_further(
        &self,
        at_time: Timestamp,
        estimated_throughput: DataRate,
    ) -> bool {
        let bitrate_reduction_interval: TimeDelta = self
            .rtt
            .clamp(TimeDelta::from_millis(10), TimeDelta::from_millis(200));
        if at_time - self.time_last_bitrate_change >= bitrate_reduction_interval {
            return true;
        }
        if self.valid_estimate() {
            let threshold = 0.5 * self.latest_estimate();
            return estimated_throughput < threshold;
        }
        false
    }

    pub fn initial_time_to_reduce_further(&self, at_time: Timestamp) -> bool {
        self.valid_estimate()
            && self.time_to_reduce_further(
                at_time,
                self.latest_estimate() / 2 - DataRate::from_bits_per_sec(1),
            )
    }

    pub fn latest_estimate(&self) -> DataRate {
        self.current_bitrate
    }

    pub fn set_rtt(&mut self, rtt: TimeDelta) {
        self.rtt = rtt;
    }

    pub fn update(&mut self, input: &RateControlInput, at_time: Timestamp) -> DataRate {
        // Set the initial bit rate value to what we're receiving the first half
        // second.
        // TODO(bugs.webrtc.org/9379): The comment above doesn't match to the code.
        if !self.bitrate_is_initialized {
            const INITIALIZATION_TIME: TimeDelta = TimeDelta::from_seconds(5);
            debug_assert!(BITRATE_WINDOW <= INITIALIZATION_TIME);
            if self.time_first_throughput_estimate.is_infinite() {
                if input.estimated_throughput.is_some() {
                    self.time_first_throughput_estimate = at_time;
                }
            } else if at_time - self.time_first_throughput_estimate > INITIALIZATION_TIME {
                if let Some(estimated_throughput) = input.estimated_throughput {
                    self.current_bitrate = estimated_throughput;
                    self.bitrate_is_initialized = true;
                }
            }
        }
        self.change_bitrate(input, at_time);
        self.current_bitrate
    }

    pub fn set_in_application_limited_region(&mut self, in_alr: bool) {
        self.in_alr = in_alr;
    }

    pub fn set_estimate(&mut self, bitrate: DataRate, at_time: Timestamp) {
        self.bitrate_is_initialized = true;
        let prev_bitrate = self.current_bitrate;
        self.current_bitrate = self.clamp_bitrate(bitrate);
        self.time_last_bitrate_change = at_time;
        if self.current_bitrate < prev_bitrate {
            self.time_last_bitrate_decrease = at_time;
        }
    }

    pub fn set_network_state_estimate(&mut self, estimate: Option<&NetworkStateEstimate>) {
        self.network_estimate = estimate.cloned();
    }

    pub fn get_near_max_increase_rate_bps_per_second(&self) -> f64 {
        assert!(!self.current_bitrate.is_zero());
        const FRAME_INTERVAL: TimeDelta = TimeDelta::from_micros(1_000_000 / 30);
        let frame_size = self.current_bitrate * FRAME_INTERVAL;
        const PACKET_SIZE: DataSize = DataSize::from_bytes(1200);
        let packets_per_frame = (frame_size.bytes() as f64 / PACKET_SIZE.bytes() as f64).ceil();
        let avg_packet_size = frame_size / packets_per_frame;
        // Approximate the over-use estimator delay to 100 ms.
        let mut response_time: TimeDelta = self.rtt + TimeDelta::from_millis(100);
        response_time = response_time * 2.0;
        let increase_rate_bps_per_second = (avg_packet_size / response_time).bps() as f64;
        const MIN_INCREASE_RATE_BPS_PER_SECOND: f64 = 4000.0;
        f64::max(
            MIN_INCREASE_RATE_BPS_PER_SECOND,
            increase_rate_bps_per_second,
        )
    }

    pub fn get_expected_bandwidth_period(&self) -> TimeDelta {
        const MIN_PERIOD: TimeDelta = TimeDelta::from_seconds(2);
        const DEFAULT_PERIOD: TimeDelta = TimeDelta::from_seconds(3);
        const MAX_PERIOD: TimeDelta = TimeDelta::from_seconds(50);
        let increase_rate_bps_per_second = self.get_near_max_increase_rate_bps_per_second();
        if self.last_decrease.is_none() {
            return DEFAULT_PERIOD;
        }
        let time_to_recover_decrease_seconds =
            self.last_decrease.unwrap().bps() as f64 / increase_rate_bps_per_second;
        let period = TimeDelta::from_seconds(time_to_recover_decrease_seconds as i64);

        TimeDelta::from_micros(period.us().clamp(MIN_PERIOD.us(), MAX_PERIOD.us()))
    }

    pub fn change_bitrate(&mut self, input: &RateControlInput, at_time: Timestamp) {
        let mut new_bitrate: Option<DataRate> = None;
        let estimated_throughput: DataRate = input
            .estimated_throughput
            .unwrap_or(self.latest_estimated_throughput);
        if let Some(estimated_throughput) = input.estimated_throughput {
            self.latest_estimated_throughput = estimated_throughput;
        }

        // An over-use should always trigger us to reduce the bitrate, even though
        // we have not yet established our first estimate. By acting on the over-use,
        // we will end up with a valid estimate.
        if !self.bitrate_is_initialized && input.bw_state != BandwidthUsage::Overusing {
            return;
        }
        self.change_state(input, at_time);
        match self.rate_control_state {
            RateControlState::Hold => (),
            RateControlState::Increase => {
                if estimated_throughput > self.link_capacity.upper_bound() {
                    self.link_capacity.reset();
                }
                // We limit the new bitrate based on the troughput to avoid unlimited
                // bitrate increases. We allow a bit more lag at very low rates to not too
                // easily get stuck if the encoder produces uneven outputs.
                let mut increase_limit =
                    1.5 * estimated_throughput + DataRate::from_kilobits_per_sec(10);
                if self.send_side && self.in_alr && self.no_bitrate_increase_in_alr {
                    // Do not increase the delay based estimate in alr since the estimator
                    // will not be able to get transport feedback necessary to detect if
                    // the new estimate is correct.
                    // If we have previously increased above the limit (for instance due to
                    // probing), we don't allow further changes.
                    increase_limit = self.current_bitrate;
                }
                if self.current_bitrate < increase_limit {
                    let increased_bitrate = if self.link_capacity.has_estimate() {
                        // The link_capacity estimate is reset if the measured throughput
                        // is too far from the estimate. We can therefore assume that our
                        // target rate is reasonably close to link capacity and use additive
                        // increase.
                        let additive_increase =
                            self.additive_rate_increase(at_time, self.time_last_bitrate_change);
                        self.current_bitrate + additive_increase
                    } else {
                        // If we don't have an estimate of the link capacity, use faster ramp
                        // up to discover the capacity.
                        let multiplicative_increase = self.multiplicative_rate_increase(
                            at_time,
                            self.time_last_bitrate_change,
                            self.current_bitrate,
                        );
                        self.current_bitrate + multiplicative_increase
                    };
                    new_bitrate = Some(std::cmp::min(increased_bitrate, increase_limit));
                }
                self.time_last_bitrate_change = at_time;
            }
            RateControlState::Decrease => {
                // Set bit rate to something slightly lower than the measured throughput
                // to get rid of any self-induced delay.
                let mut decreased_bitrate = estimated_throughput * self.beta;
                if decreased_bitrate > DataRate::from_kilobits_per_sec(5)
                    && self.subtract_additional_backoff_term
                {
                    decreased_bitrate -= DataRate::from_kilobits_per_sec(5);
                }
                if decreased_bitrate > self.current_bitrate {
                    // TODO(terelius): The link_capacity estimate may be based on old
                    // throughput measurements. Relying on them may lead to unnecessary
                    // BWE drops.
                    if self.link_capacity.has_estimate() {
                        decreased_bitrate = self.beta * self.link_capacity.estimate();
                    }
                }
                // Avoid increasing the rate when over-using.
                if decreased_bitrate < self.current_bitrate {
                    new_bitrate = Some(decreased_bitrate);
                }
                if self.bitrate_is_initialized && estimated_throughput < self.current_bitrate {
                    if let Some(bitrate) = new_bitrate {
                        self.last_decrease = Some(self.current_bitrate - bitrate);
                    } else {
                        self.last_decrease = Some(DataRate::zero());
                    }
                }
                if estimated_throughput < self.link_capacity.lower_bound() {
                    // The current throughput is far from the estimated link capacity. Clear
                    // the estimate to allow an immediate update in OnOveruseDetected.
                    self.link_capacity.reset();
                }
                self.bitrate_is_initialized = true;
                self.link_capacity.on_overuse_detected(estimated_throughput);
                // Stay on hold until the pipes are cleared.
                self.rate_control_state = RateControlState::Hold;
                self.time_last_bitrate_change = at_time;
                self.time_last_bitrate_decrease = at_time;
            }
        }
        self.current_bitrate = self.clamp_bitrate(new_bitrate.unwrap_or(self.current_bitrate));
    }

    pub fn clamp_bitrate(&self, mut new_bitrate: DataRate) -> DataRate {
        if let Some(network_estimate) = &self.network_estimate {
            if !self.disable_estimate_bounded_increase
                && network_estimate.link_capacity_upper.is_finite()
            {
                let upper_bound = if self.use_current_estimate_as_min_upper_bound {
                    std::cmp::max(network_estimate.link_capacity_upper, self.current_bitrate)
                } else {
                    network_estimate.link_capacity_upper
                };
                new_bitrate = std::cmp::min(upper_bound, new_bitrate);
            }

            if network_estimate.link_capacity_lower.is_finite()
                && new_bitrate < self.current_bitrate
            {
                new_bitrate = std::cmp::min(
                    self.current_bitrate,
                    std::cmp::max(
                        new_bitrate,
                        network_estimate.link_capacity_lower * self.beta,
                    ),
                );
            }
        }

        std::cmp::max(new_bitrate, self.min_configured_bitrate)
    }

    pub fn multiplicative_rate_increase(
        &self,
        at_time: Timestamp,
        last_time: Timestamp,
        current_bitrate: DataRate,
    ) -> DataRate {
        let mut alpha = 1.08;
        if last_time.is_finite() {
            let time_since_last_update = at_time - last_time;
            alpha = f64::powf(
                alpha,
                f64::min(time_since_last_update.seconds() as f64, 1.0),
            );
        }
        std::cmp::max(
            current_bitrate * (alpha - 1.0),
            DataRate::from_bits_per_sec(1000),
        )
    }

    pub fn additive_rate_increase(&self, at_time: Timestamp, last_time: Timestamp) -> DataRate {
        let time_period_seconds = (at_time - last_time).seconds() as f64;
        let data_rate_increase_bps =
            self.get_near_max_increase_rate_bps_per_second() * time_period_seconds;
        DataRate::from_bits_per_sec(data_rate_increase_bps as i64)
    }

    pub fn change_state(&mut self, input: &RateControlInput, at_time: Timestamp) {
        match input.bw_state {
            BandwidthUsage::Normal => {
                if self.rate_control_state == RateControlState::Hold {
                    self.time_last_bitrate_change = at_time;
                    self.rate_control_state = RateControlState::Increase;
                }
            }
            BandwidthUsage::Overusing => {
                if self.rate_control_state != RateControlState::Decrease {
                    self.rate_control_state = RateControlState::Decrease;
                }
            }

            BandwidthUsage::Underusing => {
                self.rate_control_state = RateControlState::Hold;
            }
        }
    }
}

pub struct AimdRateControlConfig {
    pub beta: f64,
    // Allow the delay based estimate to only increase as long as application
    // limited region (alr) is not detected.
    pub no_bitrate_increase_in_alr: bool,
    // If true, subtract an additional 5kbps when backing off.
    pub subtract_additional_backoff_term: bool,
    // If "Disabled",  estimated link capacity is not used as upper bound.
    pub disable_estimate_bounded_increase: bool,
    pub use_current_estimate_as_min_upper_bound: bool,
}

impl Default for AimdRateControlConfig {
    fn default() -> Self {
        Self {
            beta: DEFAULT_BACKOFF_FACTOR,
            no_bitrate_increase_in_alr: false,
            subtract_additional_backoff_term: true,
            disable_estimate_bounded_increase: false,
            use_current_estimate_as_min_upper_bound: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_abs_diff_eq;

    const INITIAL_TIME: Timestamp = Timestamp::from_millis(123_456);
    const MIN_BWE_PERIOD: TimeDelta = TimeDelta::from_seconds(2);
    const DEFAULT_PERIOD: TimeDelta = TimeDelta::from_seconds(3);
    const MAX_BWE_PERIOD: TimeDelta = TimeDelta::from_seconds(50);
    // After an overuse, we back off to 85% to the received bitrate.
    const FRACTION_AFTER_OVERUSE: f64 = 0.85;

    #[test]
    fn min_near_max_increase_rate_on_low_bandwith() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(30_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.get_near_max_increase_rate_bps_per_second(),
            4_000.0
        );
    }

    #[test]
    fn near_max_increase_rate_is_5kbps_on_90kbps_and_200ms_rtt() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(90_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.get_near_max_increase_rate_bps_per_second(),
            5_000.0
        );
    }

    #[test]
    fn near_max_increase_rate_is_5kbps_on_60kbps_and_100ms_rtt() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(60_000), INITIAL_TIME);
        aimd_rate_control.set_rtt(TimeDelta::from_millis(100));
        assert_eq!(
            aimd_rate_control.get_near_max_increase_rate_bps_per_second(),
            5_000.0
        );
    }

    #[test]
    fn get_increase_rate_and_bandwidth_period() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const BITRATE: DataRate = DataRate::from_bits_per_sec(300_000);
        aimd_rate_control.set_estimate(BITRATE, INITIAL_TIME);
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Overusing, Some(BITRATE)),
            INITIAL_TIME,
        );
        assert_abs_diff_eq!(
            aimd_rate_control.get_near_max_increase_rate_bps_per_second(),
            14_000.0,
            epsilon = 1_000.0
        );
        assert_eq!(
            aimd_rate_control.get_expected_bandwidth_period(),
            DEFAULT_PERIOD
        );
    }

    #[test]
    fn bwe_limited_by_acked_bitrate() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const ACKED_BITRATE: DataRate = DataRate::from_bits_per_sec(10_000);
        let mut now = INITIAL_TIME;
        aimd_rate_control.set_estimate(ACKED_BITRATE, now);
        while now - INITIAL_TIME < TimeDelta::from_seconds(20) {
            aimd_rate_control.update(
                &RateControlInput::new(BandwidthUsage::Normal, Some(ACKED_BITRATE)),
                now,
            );
            now += TimeDelta::from_millis(100);
        }
        assert!(aimd_rate_control.valid_estimate());
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            1.5 * ACKED_BITRATE + DataRate::from_bits_per_sec(10_000)
        );
    }

    #[test]
    fn bwe_not_limited_by_decreasing_acked_bitrate() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const ACKED_BITRATE: DataRate = DataRate::from_bits_per_sec(10_000);
        let mut now = INITIAL_TIME;
        aimd_rate_control.set_estimate(ACKED_BITRATE, now);
        while now - INITIAL_TIME < TimeDelta::from_seconds(20) {
            aimd_rate_control.update(
                &RateControlInput::new(BandwidthUsage::Normal, Some(ACKED_BITRATE)),
                now,
            );
            now += TimeDelta::from_millis(100);
        }
        assert!(aimd_rate_control.valid_estimate());
        // If the acked bitrate decreases the BWE shouldn't be reduced to 1.5x
        // what's being acked, but also shouldn't get to increase more.
        let prev_estimate = aimd_rate_control.latest_estimate();
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Normal, Some(ACKED_BITRATE / 2)),
            now,
        );
        let new_estimate = aimd_rate_control.latest_estimate();
        assert_eq!(new_estimate, prev_estimate);
        assert_abs_diff_eq!(
            new_estimate.bps() as f64,
            (1.5 * ACKED_BITRATE + DataRate::from_bits_per_sec(10_000)).bps() as f64,
            epsilon = 2_000.0
        );
    }

    #[test]
    fn default_period_until_first_overuse() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        aimd_rate_control.set_start_bitrate(DataRate::from_kilobits_per_sec(300));
        assert_eq!(
            aimd_rate_control.get_expected_bandwidth_period(),
            DEFAULT_PERIOD
        );
        aimd_rate_control.update(
            &RateControlInput::new(
                BandwidthUsage::Overusing,
                Some(DataRate::from_kilobits_per_sec(280)),
            ),
            INITIAL_TIME,
        );
        assert_ne!(
            aimd_rate_control.get_expected_bandwidth_period(),
            DEFAULT_PERIOD
        );
    }

    #[test]
    fn expected_period_after_typical_drop() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        // The rate increase at 216 kbps should be 12 kbps. If we drop from
        // 216 + 4*12 = 264 kbps, it should take 4 seconds to recover. Since we
        // back off to 0.85*acked_rate-5kbps, the acked bitrate needs to be 260
        // kbps to end up at 216 kbps.
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(264_000);
        const UPDATED_BITRATE: DataRate = DataRate::from_bits_per_sec(216_000);
        let acked_bitrate =
            (UPDATED_BITRATE + DataRate::from_bits_per_sec(5_000)) / FRACTION_AFTER_OVERUSE;
        let mut now = INITIAL_TIME;
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        now += TimeDelta::from_millis(100);
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Overusing, Some(acked_bitrate)),
            now,
        );
        assert_eq!(aimd_rate_control.latest_estimate(), UPDATED_BITRATE);
        assert_eq!(
            aimd_rate_control.get_near_max_increase_rate_bps_per_second(),
            12_000.0
        );
        assert_eq!(
            aimd_rate_control.get_expected_bandwidth_period(),
            TimeDelta::from_seconds(4)
        );
    }

    #[test]
    fn bandwidth_period_is_not_below_min() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(10_000);
        let mut now = INITIAL_TIME;
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        now += TimeDelta::from_millis(100);
        // Make a small (1.5 kbps) bitrate drop to 8.5 kbps.
        aimd_rate_control.update(
            &RateControlInput::new(
                BandwidthUsage::Overusing,
                Some(INITIAL_BITRATE - DataRate::from_bits_per_sec(1)),
            ),
            now,
        );
        assert_eq!(
            aimd_rate_control.get_expected_bandwidth_period(),
            MIN_BWE_PERIOD
        );
    }

    #[test]
    fn bandwidth_period_is_not_above_max_no_smoothing_exp() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(10_010_000);
        let mut now = INITIAL_TIME;
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        now += TimeDelta::from_millis(100);
        // Make a large (10 Mbps) bitrate drop to 10 kbps.
        let acked_bitrate = DataRate::from_bits_per_sec(10_000) / FRACTION_AFTER_OVERUSE;
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Overusing, Some(acked_bitrate)),
            now,
        );
        assert_eq!(
            aimd_rate_control.get_expected_bandwidth_period(),
            MAX_BWE_PERIOD
        );
    }

    #[test]
    fn sending_rate_bounded_when_throughput_not_estimated() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), false);
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(123_000);
        let mut now = INITIAL_TIME;
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Normal, Some(INITIAL_BITRATE)),
            now,
        );
        // AimdRateControl sets the initial bit rate to what it receives after
        // five seconds has passed.
        // TODO(bugs.webrtc.org/9379): The comment in the AimdRateControl does not
        // match the constant.
        const INITIALIZATION_TIME: TimeDelta = TimeDelta::from_seconds(5);
        now += INITIALIZATION_TIME + TimeDelta::from_millis(1);
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Normal, Some(INITIAL_BITRATE)),
            now,
        );
        for _ in 0..100 {
            aimd_rate_control.update(&RateControlInput::new(BandwidthUsage::Normal, None), now);
            now += TimeDelta::from_millis(100);
        }
        assert!(
            aimd_rate_control.latest_estimate()
                <= INITIAL_BITRATE * 1.5 + DataRate::from_bits_per_sec(10_000)
        );
    }

    #[test]
    fn estimate_does_not_increase_in_alr() {
        // When alr is detected, the delay based estimator is not allowed to increase
        // bwe since there will be no feedback from the network if the new estimate
        // is correct.
        let mut aimd_rate_control = AimdRateControl::new(
            AimdRateControlConfig {
                no_bitrate_increase_in_alr: true,
                ..Default::default()
            },
            true,
        );
        let mut now = INITIAL_TIME;
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(123_000);
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        aimd_rate_control.set_in_application_limited_region(true);
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Normal, Some(INITIAL_BITRATE)),
            now,
        );
        assert_eq!(aimd_rate_control.latest_estimate(), INITIAL_BITRATE);
        for _ in 0..100 {
            aimd_rate_control.update(&RateControlInput::new(BandwidthUsage::Normal, None), now);
            now += TimeDelta::from_millis(100);
        }
        assert_eq!(aimd_rate_control.latest_estimate(), INITIAL_BITRATE);
    }

    #[test]
    fn set_estimate_increase_bwe_in_alr() {
        let mut aimd_rate_control = AimdRateControl::new(
            AimdRateControlConfig {
                no_bitrate_increase_in_alr: true,
                ..Default::default()
            },
            true,
        );
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(123_000);
        aimd_rate_control.set_estimate(INITIAL_BITRATE, INITIAL_TIME);
        aimd_rate_control.set_in_application_limited_region(true);
        assert_eq!(aimd_rate_control.latest_estimate(), INITIAL_BITRATE);
        aimd_rate_control.set_estimate(2.0 * INITIAL_BITRATE, INITIAL_TIME);
        assert_eq!(aimd_rate_control.latest_estimate(), 2.0 * INITIAL_BITRATE);
    }

    #[test]
    fn set_estimate_upper_limited_by_network_estimate() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), true);
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(300_000), INITIAL_TIME);
        let network_estimate = NetworkStateEstimate {
            link_capacity_upper: DataRate::from_bits_per_sec(400_000),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(500_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            network_estimate.link_capacity_upper
        );
    }

    #[test]
    fn set_estimate_default_upper_limited_by_current_bitrate_if_network_estimate_is_low() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), true);
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(500_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            DataRate::from_bits_per_sec(500_000)
        );
        let network_estimate = NetworkStateEstimate {
            link_capacity_upper: DataRate::from_bits_per_sec(300_000),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(700_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            DataRate::from_bits_per_sec(500_000)
        );
    }

    #[test]
    fn set_estimate_not_upper_limited_by_current_bitrate_if_network_estimate_is_low_if() {
        let mut aimd_rate_control = AimdRateControl::new(
            AimdRateControlConfig {
                use_current_estimate_as_min_upper_bound: false,
                ..Default::default()
            },
            true,
        );
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(500_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            DataRate::from_bits_per_sec(500_000)
        );
        let network_estimate = NetworkStateEstimate {
            link_capacity_upper: DataRate::from_bits_per_sec(300_000),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(700_000), INITIAL_TIME);
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            DataRate::from_bits_per_sec(300_000)
        );
    }

    #[test]
    fn set_estimate_lower_limited_by_network_estimate() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), true);
        let network_estimate = NetworkStateEstimate {
            link_capacity_lower: DataRate::from_bits_per_sec(400_000),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        aimd_rate_control.set_estimate(DataRate::from_bits_per_sec(100_000), INITIAL_TIME);
        // 0.85 is default backoff factor. (`beta_`)
        assert_eq!(
            aimd_rate_control.latest_estimate(),
            network_estimate.link_capacity_lower * 0.85
        );
    }

    #[test]
    fn set_estimate_ignored_if_lower_than_network_estimate_and_current() {
        let mut aimd_rate_control = AimdRateControl::new(AimdRateControlConfig::default(), true);
        aimd_rate_control.set_estimate(DataRate::from_kilobits_per_sec(200), INITIAL_TIME);
        assert_eq!(aimd_rate_control.latest_estimate().kbps(), 200);
        let network_estimate = NetworkStateEstimate {
            link_capacity_lower: DataRate::from_kilobits_per_sec(400),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        // Ignore the next SetEstimate, since the estimate is lower than 85% of
        // the network estimate.
        aimd_rate_control.set_estimate(DataRate::from_kilobits_per_sec(100), INITIAL_TIME);
        assert_eq!(aimd_rate_control.latest_estimate().kbps(), 200);
    }

    #[test]
    fn estimate_increase_while_not_in_alr() {
        // Allow the estimate to increase as long as alr is not detected to ensure
        // tha BWE can not get stuck at a certain bitrate.
        let mut aimd_rate_control = AimdRateControl::new(
            AimdRateControlConfig {
                no_bitrate_increase_in_alr: true,
                ..Default::default()
            },
            true,
        );
        let mut now = INITIAL_TIME;
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(123_000);
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        aimd_rate_control.set_in_application_limited_region(false);
        aimd_rate_control.update(
            &RateControlInput::new(BandwidthUsage::Normal, Some(INITIAL_BITRATE)),
            now,
        );
        for _ in 0..100 {
            aimd_rate_control.update(&RateControlInput::new(BandwidthUsage::Normal, None), now);
            now += TimeDelta::from_millis(100);
        }
        assert!(aimd_rate_control.latest_estimate() > INITIAL_BITRATE);
    }

    #[test]
    fn estimate_not_limited_by_network_estimate_if_disabled() {
        let mut aimd_rate_control = AimdRateControl::new(
            AimdRateControlConfig {
                disable_estimate_bounded_increase: true,
                ..Default::default()
            },
            true,
        );
        let mut now = INITIAL_TIME;
        const INITIAL_BITRATE: DataRate = DataRate::from_bits_per_sec(123_000);
        aimd_rate_control.set_estimate(INITIAL_BITRATE, now);
        aimd_rate_control.set_in_application_limited_region(false);
        let network_estimate = NetworkStateEstimate {
            link_capacity_upper: DataRate::from_kilobits_per_sec(150),
            ..Default::default()
        };
        aimd_rate_control.set_network_state_estimate(Some(&network_estimate));
        for _ in 0..100 {
            aimd_rate_control.update(&RateControlInput::new(BandwidthUsage::Normal, None), now);
            now += TimeDelta::from_millis(100);
        }
        assert!(aimd_rate_control.latest_estimate() > network_estimate.link_capacity_upper);
    }
}
