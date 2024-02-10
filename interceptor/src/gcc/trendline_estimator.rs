use super::{aimd_rate_control::BandwidthUsage, network_state_predictor::NetworkStatePredictor};
use std::collections::VecDeque;

// Parameters for linear least squares fit of regression line to noisy data.
const DEFAULT_TRENDLINE_SMOOTHING_COEFF: f64 = 0.9;
const DEFAULT_TRENDLINE_THRESHOLD_GAIN: f64 = 4.0;

const MAX_ADAPT_OFFSET_MS: f64 = 15.0;
const OVER_USING_TIME_THRESHOLD: f64 = 10.0;
const MIN_NUM_DELTAS: i32 = 60;
const DELTA_COUNTER_MAX: i32 = 1000;

const DEFAULT_TRENDLINE_WINDOW_SIZE: u32 = 20;

const TIME_OVER_USING_UNDEFINED: f64 = -1.0;

pub struct TrendlineEstimator {
    // Parameters.
    settings: TrendlineEstimatorSettings,
    smoothing_coef: f64,
    threshold_gain: f64,
    // Used by the existing threshold.
    num_of_deltas: i32,
    // Keep the arrival times small by using the change from the first packet.
    first_arrival_time_ms: i64,
    // Exponential backoff filtering.
    accumulated_delay: f64,
    smoothed_delay: f64,
    // Linear least squares regression.
    delay_hist: VecDeque<PacketTiming>,
    k_up: f64,
    k_down: f64,
    overusing_time_threshold: f64,
    threshold: f64,
    prev_modified_trend: f64,
    last_update_ms: i64,
    prev_trend: f64,
    time_over_using: f64,
    overuse_counter: i32,
    hypothesis: BandwidthUsage,
    hypothesis_predicted: BandwidthUsage,
    network_state_predictor: Option<Box<dyn NetworkStatePredictor>>,
}

fn linear_fit_slope(packets: &VecDeque<PacketTiming>) -> Option<f64> {
    debug_assert!(packets.len() >= 2);
    // Compute the "center of mass".
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    for packet in packets {
        sum_x += packet.arrival_time_ms;
        sum_y += packet.smoothed_delay_ms;
    }

    let x_avg = sum_x / packets.len() as f64;
    let y_avg = sum_y / packets.len() as f64;
    // Compute the slope k = \sum (x_i-x_avg)(y_i-y_avg) / \sum (x_i-x_avg)^2
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for packet in packets {
        let x = packet.arrival_time_ms;
        let y = packet.smoothed_delay_ms;
        numerator += (x - x_avg) * (y - y_avg);
        denominator += (x - x_avg) * (x - x_avg);
    }
    if denominator == 0.0 {
        None
    } else {
        Some(numerator / denominator)
    }
}

fn compute_slope_cap(
    packets: &VecDeque<PacketTiming>,
    settings: &TrendlineEstimatorSettings,
) -> Option<f64> {
    debug_assert!(
        1 <= settings.beginning_packets && (settings.beginning_packets as usize) < packets.len()
    );
    debug_assert!(1 <= settings.end_packets && (settings.end_packets as usize) < packets.len());
    debug_assert!((settings.beginning_packets + settings.end_packets) as usize <= packets.len());

    let mut early = packets[0].clone();
    for packet in packets
        .iter()
        .take(settings.beginning_packets as usize)
        .skip(1)
    {
        if packet.raw_delay_ms < early.raw_delay_ms {
            early = packet.clone();
        }
    }

    let late_start = packets.len() - settings.end_packets as usize;
    let mut late = packets[late_start].clone();
    for packet in packets.iter().skip(late_start + 1) {
        if packet.raw_delay_ms < late.raw_delay_ms {
            late = packet.clone();
        }
    }
    if late.arrival_time_ms - early.arrival_time_ms < 1.0 {
        None
    } else {
        Some(
            (late.raw_delay_ms - early.raw_delay_ms)
                / (late.arrival_time_ms - early.arrival_time_ms)
                + settings.cap_uncertainty,
        )
    }
}

impl TrendlineEstimator {
    pub fn new(
        settings: TrendlineEstimatorSettings,
        network_state_predictor: Option<Box<dyn NetworkStatePredictor>>,
    ) -> Self {
        // ```
        // if self.delay_hist.len() > self.window_size {
        //     self.delay_hist.pop_front();
        // }
        // ```
        // Would permit `self.delay_hist.len()` to be 1 greater than `self.settings.window_size`
        let delay_hist = VecDeque::with_capacity(settings.window_size as usize + 1);

        Self {
            settings,
            smoothing_coef: DEFAULT_TRENDLINE_SMOOTHING_COEFF,
            threshold_gain: DEFAULT_TRENDLINE_THRESHOLD_GAIN,
            num_of_deltas: 0,
            first_arrival_time_ms: -1,
            accumulated_delay: 0.0,
            smoothed_delay: 0.0,
            delay_hist,
            k_up: 0.0087,
            k_down: 0.039,
            overusing_time_threshold: OVER_USING_TIME_THRESHOLD,
            threshold: 12.5,
            prev_modified_trend: f64::NAN,
            last_update_ms: -1,
            prev_trend: 0.0,
            time_over_using: TIME_OVER_USING_UNDEFINED,
            overuse_counter: 0,
            hypothesis: BandwidthUsage::Normal,
            hypothesis_predicted: BandwidthUsage::Normal,
            network_state_predictor,
        }
    }

    pub fn update_trendline(
        &mut self,
        recv_delta_ms: f64,
        send_delta_ms: f64,
        _send_time_ms: i64,
        arrival_time_ms: i64,
        _packet_size: usize,
    ) {
        let delta_ms = recv_delta_ms - send_delta_ms;
        self.num_of_deltas += 1;
        self.num_of_deltas = std::cmp::min(self.num_of_deltas, DELTA_COUNTER_MAX);
        if self.first_arrival_time_ms == -1 {
            self.first_arrival_time_ms = arrival_time_ms;
        }
        // Exponential backoff filter.
        self.accumulated_delay += delta_ms;
        self.smoothed_delay = self.smoothing_coef * self.smoothed_delay
            + (1.0 - self.smoothing_coef) * self.accumulated_delay;
        // Maintain packet window
        self.delay_hist.push_back(PacketTiming::new(
            (arrival_time_ms - self.first_arrival_time_ms) as f64,
            self.smoothed_delay,
            self.accumulated_delay,
        ));
        if self.settings.enable_sort {
            for i in (1..self.delay_hist.len()).rev() {
                if self.delay_hist[i].arrival_time_ms < self.delay_hist[i - 1].arrival_time_ms {
                    self.delay_hist.swap(i, i - 1);
                } else {
                    break;
                }
            }
        }
        if self.delay_hist.len() > self.settings.window_size as usize {
            self.delay_hist.pop_front();
        }
        // Simple linear regression.
        let mut trend = self.prev_trend;
        if self.delay_hist.len() == self.settings.window_size as usize {
            // Update `self.trend` if it is possible to fit a line to the data. The delay
            // trend can be seen as an estimate of (send_rate - capacity)/capacity.
            // 0 < trend < 1   ->  the delay increases, queues are filling up
            //   trend == 0    ->  the delay does not change
            //   trend < 0     ->  the delay decreases, queues are being emptied
            trend = linear_fit_slope(&self.delay_hist).unwrap_or(trend);
            if self.settings.enable_cap {
                // We only use the cap to filter out overuse detections, not
                // to detect additional underuses.
                if let Some(cap) = compute_slope_cap(&self.delay_hist, &self.settings) {
                    if trend >= 0.0 && trend > cap {
                        trend = cap;
                    }
                }
            }
        }

        self.detect(trend, send_delta_ms, arrival_time_ms);
    }

    /// Update the estimator with a new sample. The deltas should represent deltas
    /// between timestamp groups as defined by the InterArrival class.
    pub fn update(
        &mut self,
        recv_delta_ms: f64,
        send_delta_ms: f64,
        send_time_ms: i64,
        arrival_time_ms: i64,
        packet_size: usize,
        calculated_deltas: bool,
    ) {
        if calculated_deltas {
            self.update_trendline(
                recv_delta_ms,
                send_delta_ms,
                send_time_ms,
                arrival_time_ms,
                packet_size,
            );
        }
        if let Some(network_state_predictor) = &mut self.network_state_predictor {
            self.hypothesis_predicted =
                network_state_predictor.update(send_time_ms, arrival_time_ms, self.hypothesis);
        }
    }

    pub fn state(&self) -> BandwidthUsage {
        if self.network_state_predictor.is_some() {
            self.hypothesis_predicted
        } else {
            self.hypothesis
        }
    }

    fn detect(&mut self, trend: f64, ts_delta: f64, now_ms: i64) {
        if self.num_of_deltas < 2 {
            self.hypothesis = BandwidthUsage::Normal;
            return;
        }
        let modified_trend =
            std::cmp::min(self.num_of_deltas, MIN_NUM_DELTAS) as f64 * trend * self.threshold_gain;
        self.prev_modified_trend = modified_trend;
        if modified_trend > self.threshold {
            if self.time_over_using == TIME_OVER_USING_UNDEFINED {
                // Initialize the timer. Assume that we've been
                // over-using half of the time since the previous
                // sample.
                self.time_over_using = ts_delta / 2.0;
            } else {
                // Increment timer
                self.time_over_using += ts_delta;
            }
            self.overuse_counter += 1;

            #[allow(clippy::collapsible_if)]
            if self.time_over_using > self.overusing_time_threshold && self.overuse_counter > 1 {
                if trend >= self.prev_trend {
                    self.time_over_using = 0.0;
                    self.overuse_counter = 0;
                    self.hypothesis = BandwidthUsage::Overusing;
                }
            }
        } else if modified_trend < -self.threshold {
            self.time_over_using = TIME_OVER_USING_UNDEFINED;
            self.overuse_counter = 0;
            self.hypothesis = BandwidthUsage::Underusing;
        } else {
            self.time_over_using = TIME_OVER_USING_UNDEFINED;
            self.overuse_counter = 0;
            self.hypothesis = BandwidthUsage::Normal;
        }
        self.prev_trend = trend;
        self.update_threshold(modified_trend, now_ms);
    }

    fn update_threshold(&mut self, modified_trend: f64, now_ms: i64) {
        if self.last_update_ms == -1 {
            self.last_update_ms = now_ms;
        }
        if modified_trend.abs() > self.threshold + MAX_ADAPT_OFFSET_MS {
            // Avoid adapting the threshold to big latency spikes, caused e.g.,
            // by a sudden capacity drop.
            self.last_update_ms = now_ms;
            return;
        }
        let k = if modified_trend.abs() < self.threshold {
            self.k_down
        } else {
            self.k_up
        };
        const MAX_TIME_DELTA_MS: i64 = 100;
        let time_delta_ms = std::cmp::min(now_ms - self.last_update_ms, MAX_TIME_DELTA_MS);
        self.threshold += k * (modified_trend.abs() - self.threshold) * time_delta_ms as f64;
        self.threshold = self.threshold.clamp(6.0, 600.0);
        self.last_update_ms = now_ms;
    }
}

pub struct TrendlineEstimatorSettings {
    // Sort the packets in the window. Should be redundant,
    // but then almost no cost.
    enable_sort: bool,
    // Cap the trendline slope based on the minimum delay seen
    // in the beginning_packets and end_packets respectively.
    enable_cap: bool,
    beginning_packets: u32,
    end_packets: u32,
    cap_uncertainty: f64,
    // Size (in packets) of the window.
    window_size: u32,
}

impl Default for TrendlineEstimatorSettings {
    fn default() -> Self {
        Self {
            enable_sort: false,
            enable_cap: false,
            beginning_packets: 7,
            end_packets: 7,
            cap_uncertainty: 0.0,
            window_size: DEFAULT_TRENDLINE_WINDOW_SIZE,
        }
    }
}

#[derive(Debug, Clone)]
struct PacketTiming {
    arrival_time_ms: f64,
    smoothed_delay_ms: f64,
    raw_delay_ms: f64,
}

impl PacketTiming {
    fn new(arrival_time_ms: f64, smoothed_delay_ms: f64, raw_delay_ms: f64) -> Self {
        Self {
            arrival_time_ms,
            smoothed_delay_ms,
            raw_delay_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET_COUNT: usize = 25;
    const PACKET_SIZE_BYTES: usize = 1200;

    struct PacketTimeGenerator {
        initial_clock: i64,
        time_between_packets: f64,
        packets: usize,
    }

    impl PacketTimeGenerator {
        fn new(initial_clock: i64, time_between_packets: f64) -> Self {
            Self {
                initial_clock,
                time_between_packets,
                packets: 0,
            }
        }

        fn next(&mut self) -> i64 {
            let val = self.initial_clock as f64 + self.time_between_packets * self.packets as f64;
            self.packets += 1;
            val as i64
        }

        fn generate(&mut self, vec: &mut Vec<i64>) {
            for item in vec {
                *item = self.next();
            }
        }
    }

    struct TrendlineEstimatorTest {
        send_times: Vec<i64>,
        recv_times: Vec<i64>,
        packet_sizes: Vec<usize>,
        estimator: TrendlineEstimator,
        count: usize,
    }

    impl TrendlineEstimatorTest {
        fn new() -> Self {
            Self {
                send_times: vec![Default::default(); PACKET_COUNT],
                recv_times: vec![Default::default(); PACKET_COUNT],
                packet_sizes: vec![PACKET_SIZE_BYTES; PACKET_COUNT],
                estimator: TrendlineEstimator::new(Default::default(), None),
                count: 1,
            }
        }

        fn run_test_until_state_change(&mut self) {
            debug_assert_eq!(self.send_times.len(), PACKET_COUNT);
            debug_assert_eq!(self.recv_times.len(), PACKET_COUNT);
            debug_assert_eq!(self.packet_sizes.len(), PACKET_COUNT);
            debug_assert!(self.count >= 1);
            debug_assert!(self.count < PACKET_COUNT);
            let initial_state = self.estimator.state();
            while self.count < PACKET_COUNT {
                let recv_delta = self.recv_times[self.count] - self.recv_times[self.count - 1];
                let send_delta = self.send_times[self.count] - self.send_times[self.count - 1];
                self.estimator.update(
                    recv_delta as f64,
                    send_delta as f64,
                    self.send_times[self.count],
                    self.recv_times[self.count],
                    self.packet_sizes[self.count],
                    true,
                );
                if self.estimator.state() != initial_state {
                    return;
                }
                self.count += 1;
            }
        }
    }

    #[test]
    fn normal() {
        let mut test = TrendlineEstimatorTest::new();
        let mut send_time_generator = PacketTimeGenerator::new(
            123456789, /*initial clock*/
            20.0,      /*20 ms between sent packets*/
        );
        send_time_generator.generate(&mut test.send_times);
        let mut recv_time_generator = PacketTimeGenerator::new(
            987654321, /*initial clock*/
            20.0,      /*delivered at the same pace*/
        );
        recv_time_generator.generate(&mut test.recv_times);
        assert_eq!(test.estimator.state(), BandwidthUsage::Normal);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Normal);
        assert_eq!(test.count, PACKET_COUNT); // All packets processed
    }

    #[test]
    fn overusing() {
        let mut test = TrendlineEstimatorTest::new();
        let mut send_time_generator = PacketTimeGenerator::new(
            123456789, /*initial clock*/
            20.0,      /*20 ms between sent packets*/
        );
        send_time_generator.generate(&mut test.send_times);
        let mut recv_time_generator = PacketTimeGenerator::new(
            987654321,  /*initial clock*/
            1.1 * 20.0, /*10% slower delivery*/
        );
        recv_time_generator.generate(&mut test.recv_times);
        assert_eq!(test.estimator.state(), BandwidthUsage::Normal);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Overusing);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Overusing);
        assert_eq!(test.count, PACKET_COUNT); // All packets processed
    }

    #[test]
    fn underusing() {
        let mut test = TrendlineEstimatorTest::new();
        let mut send_time_generator = PacketTimeGenerator::new(
            123456789, /*initial clock*/
            20.0,      /*20 ms between sent packets*/
        );
        send_time_generator.generate(&mut test.send_times);
        let mut recv_time_generator = PacketTimeGenerator::new(
            987654321,   /*initial clock*/
            0.85 * 20.0, /*15% faster delivery*/
        );
        recv_time_generator.generate(&mut test.recv_times);
        assert_eq!(test.estimator.state(), BandwidthUsage::Normal);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Underusing);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Underusing);
        assert_eq!(test.count, PACKET_COUNT); // All packets processed
    }

    #[test]
    fn includes_small_packets_by_default() {
        let mut test = TrendlineEstimatorTest::new();
        let mut send_time_generator = PacketTimeGenerator::new(
            123456789, /*initial clock*/
            20.0,      /*20 ms between sent packets*/
        );
        send_time_generator.generate(&mut test.send_times);
        let mut recv_time_generator = PacketTimeGenerator::new(
            987654321,  /*initial clock*/
            1.1 * 20.0, /*10% slower delivery*/
        );
        recv_time_generator.generate(&mut test.recv_times);
        test.packet_sizes.fill(100);
        assert_eq!(test.estimator.state(), BandwidthUsage::Normal);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Overusing);
        test.run_test_until_state_change();
        assert_eq!(test.estimator.state(), BandwidthUsage::Overusing);
        assert_eq!(test.count, PACKET_COUNT); // All packets processed
    }
}
