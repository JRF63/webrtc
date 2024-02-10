use super::{
    aimd_rate_control::{AimdRateControl, BandwidthUsage},
    data_rate::DataRate,
    inter_arrival::InterArrival,
    inter_arrival_delta::InterArrivalDelta,
    network_state_predictor::NetworkStatePredictor,
    time::{TimeDelta, Timestamp},
    trendline_estimator::TrendlineEstimator,
};

pub struct DelayBasedBwe {
    separate_audio_: BweSeparateAudioPacketsSettings,
    audio_packets_since_last_video_: i64,
    last_video_packet_recv_time_: Timestamp,
    network_state_predictor_: Box<dyn NetworkStatePredictor>,
    video_inter_arrival_: InterArrival,
    video_inter_arrival_delta_: InterArrivalDelta,
    video_delay_detector_: TrendlineEstimator,
    audio_inter_arrival_: InterArrival,
    audio_inter_arrival_delta_: InterArrivalDelta,
    audio_delay_detector_: TrendlineEstimator,
    active_delay_detector_: TrendlineEstimator,
    last_seen_packet_: Timestamp,
    uma_recorded_: bool,
    rate_control_: AimdRateControl,
    prev_bitrate_: DataRate,
    prev_state_: BandwidthUsage,
}

pub struct BweSeparateAudioPacketsSettings {
    enabled: bool,
    packet_threshold: i32,
    time_threshold: TimeDelta,
}

impl BweSeparateAudioPacketsSettings {
    pub fn new() -> Self {
        Self {
            enabled: false,
            packet_threshold: 10,
            time_threshold: TimeDelta::from_seconds(1),
        }
    }
}
