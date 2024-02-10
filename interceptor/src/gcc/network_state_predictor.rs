use super::aimd_rate_control::BandwidthUsage;

pub trait NetworkStatePredictor {
    fn update(
        &mut self,
        send_time_ms: i64,
        arrival_time_ms: i64,
        network_state: BandwidthUsage,
    ) -> BandwidthUsage;
}
