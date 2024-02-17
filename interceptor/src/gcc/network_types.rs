// TODO: Maybe move to aimd_rate_control.rs?

use super::{
    data_rate::{DataRate, DataSize},
    time::{TimeDelta, Timestamp},
};

#[derive(Debug, Clone)]
pub struct PacedPacketInfo {
    send_bitrate: DataRate,
    probe_cluster_id: i32,
    probe_cluster_min_probes: i32,
    probe_cluster_min_bytes: i32,
    probe_cluster_bytes_sent: i32,
}

impl PacedPacketInfo {
    const NOT_A_PROBE: i32 = -1;
}

impl Default for PacedPacketInfo {
    fn default() -> Self {
        Self {
            send_bitrate: DataRate::zero(),
            probe_cluster_id: PacedPacketInfo::NOT_A_PROBE,
            probe_cluster_min_probes: -1,
            probe_cluster_min_bytes: -1,
            probe_cluster_bytes_sent: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SentPacket {
    send_time: Timestamp,
    // Size of packet with overhead up to IP layer.
    size: DataSize,
    // Size of preceeding packets that are not part of feedback.
    prior_unacked_data: DataSize,
    // Probe cluster id and parameters including bitrate, number of packets and
    // number of bytes.
    pacing_info: PacedPacketInfo,
    // True if the packet is an audio packet, false for video, padding, RTX etc.
    audio: bool,
    // Transport independent sequence number, any tracked packet should have a
    // sequence number that is unique over the whole call and increasing by 1 for
    // each packet.
    sequence_number: i64,
    // Tracked data in flight when the packet was sent, excluding unacked data.
    data_in_flight: DataSize,
}

impl Default for SentPacket {
    fn default() -> Self {
        Self {
            send_time: Timestamp::plus_infinity(),
            size: DataSize::zero(),
            prior_unacked_data: DataSize::zero(),
            pacing_info: Default::default(),
            audio: false,
            sequence_number: 0,
            data_in_flight: DataSize::zero(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PacketResult {
    sent_packet: SentPacket,
    receive_time: Timestamp,
}

impl Default for PacketResult {
    fn default() -> Self {
        Self {
            sent_packet: Default::default(),
            receive_time: Timestamp::plus_infinity(),
        }
    }
}

impl PacketResult {
    pub fn is_received(&self) -> bool {
        !self.receive_time.is_plus_infinity()
    }
}

pub struct TransportPacketsFeedback {
    feedback_time: Timestamp,
    first_unacked_send_time: Timestamp,
    data_in_flight: DataSize,
    prior_in_flight: DataSize,
    packet_feedbacks: Vec<PacketResult>,
    // Arrival times for messages without send time information.
    sendless_arrival_times: Vec<Timestamp>,
}

impl Default for TransportPacketsFeedback {
    fn default() -> Self {
        Self {
            feedback_time: Timestamp::plus_infinity(),
            first_unacked_send_time: Timestamp::plus_infinity(),
            data_in_flight: DataSize::zero(),
            prior_in_flight: DataSize::zero(),
            packet_feedbacks: Vec::new(),
            sendless_arrival_times: Vec::new(),
        }
    }
}

impl TransportPacketsFeedback {
    pub fn received_with_send_info(&self) -> Vec<PacketResult> {
        self.packet_feedbacks
            .iter()
            .filter(|fb| fb.is_received())
            .cloned()
            .collect()
    }

    pub fn lost_with_send_info(&self) -> Vec<PacketResult> {
        self.packet_feedbacks
            .iter()
            .filter(|fb| !fb.is_received())
            .cloned()
            .collect()
    }

    pub fn packets_with_feedback(&self) -> Vec<PacketResult> {
        self.packet_feedbacks.clone()
    }

    pub fn sorted_by_receive_time(&self) -> Vec<PacketResult> {
        let mut res = self.received_with_send_info();

        // https://webrtc.googlesource.com/src/+/0c4165e667751972c7d39c81d8993e8617cb7e13/api/transport/network_types.cc#33
        res.sort_by_key(|k| {
            (
                k.receive_time,
                k.sent_packet.send_time,
                k.sent_packet.sequence_number,
            )
        });

        res
    }
}

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
