// TODO: Maybe move to aimd_rate_control.rs?

use super::{
    data_rate::DataRate,
    time::{TimeDelta, Timestamp},
};

#[derive(Clone)]
pub struct NetworkStateEstimate {
    pub confidence: f64,
    // The time the estimate was received/calculated.
    pub update_time: Timestamp,
    pub last_receive_time: Timestamp,
    pub last_send_time: Timestamp,
    // Total estimated link capacity.
    pub link_capacity: DataRate,
    // Used as a safe measure of available capacity.
    pub link_capacity_lower: DataRate,
    // Used as limit for increasing bitrate.
    pub link_capacity_upper: DataRate,
    pub pre_link_buffer_delay: TimeDelta,
    pub post_link_buffer_delay: TimeDelta,
    pub propagation_delay: TimeDelta,
    // Only for debugging
    #[cfg(debug_assertions)]
    pub debug: NetworkStateEstimateDebug,
}

impl Default for NetworkStateEstimate {
    fn default() -> Self {
        Self {
            confidence: f64::NAN,
            update_time: Timestamp::minus_infinity(),
            last_receive_time: Timestamp::minus_infinity(),
            last_send_time: Timestamp::minus_infinity(),
            link_capacity: DataRate::minus_infinity(),
            link_capacity_lower: DataRate::minus_infinity(),
            link_capacity_upper: DataRate::minus_infinity(),
            pre_link_buffer_delay: TimeDelta::minus_infinity(),
            post_link_buffer_delay: TimeDelta::minus_infinity(),
            propagation_delay: TimeDelta::minus_infinity(),
            #[cfg(debug_assertions)]
            debug: NetworkStateEstimateDebug::default(),
        }
    }
}

#[cfg(debug_assertions)]
#[derive(Clone)]
pub struct NetworkStateEstimateDebug {
    time_delta: TimeDelta,
    last_feed_time: Timestamp,
    cross_delay_rate: f64,
    spike_delay_rate: f64,
    link_capacity_std_dev: DataRate,
    link_capacity_min: DataRate,
    cross_traffic_ratio: f64,
}

#[cfg(debug_assertions)]
impl Default for NetworkStateEstimateDebug {
    fn default() -> Self {
        Self {
            time_delta: TimeDelta::minus_infinity(),
            last_feed_time: Timestamp::minus_infinity(),
            cross_delay_rate: f64::NAN,
            spike_delay_rate: f64::NAN,
            link_capacity_std_dev: DataRate::minus_infinity(),
            link_capacity_min: DataRate::minus_infinity(),
            cross_traffic_ratio: f64::NAN,
        }
    }
}
