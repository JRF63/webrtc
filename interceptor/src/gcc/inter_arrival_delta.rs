use super::time::{TimeDelta, Timestamp};

// After this many packet groups received out of order InterArrival will reset, assuming that clocks have made a jump.
const REORDERED_RESET_THRESHOLD: i32 = 3;
const ARRIVAL_TIME_OFFSET_THRESHOLD: TimeDelta = TimeDelta::from_seconds(3);

const BURST_DELTA_THRESHOLD: TimeDelta = TimeDelta::from_millis(5);
const MAX_BURST_DURATION: TimeDelta = TimeDelta::from_millis(100);

/// Helper class to compute the inter-arrival time delta and the size delta between two send
/// bursts. This code is branched from modules/remote_bitrate_estimator/inter_arrival.
pub struct InterArrivalDelta {
    send_time_group_length: TimeDelta,
    current_timestamp_group: SendTimeGroup,
    prev_timestamp_group: SendTimeGroup,
    num_consecutive_reordered_packets: i32,
}

impl InterArrivalDelta {
    /// A send time group is defined as all packets with a send time which are at most
    /// send_time_group_length older than the first timestamp in that group.
    pub fn new(send_time_group_length: TimeDelta) -> Self {
        Self {
            send_time_group_length,
            current_timestamp_group: SendTimeGroup::new(),
            prev_timestamp_group: SendTimeGroup::new(),
            num_consecutive_reordered_packets: 0,
        }
    }

    /// This function returns true if a delta was computed, or false if the current group is still
    /// incomplete or if only one group has been completed.
    ///
    /// -`send_time` is the send time.
    /// -`arrival_time` is the time at which the packet arrived.
    /// -`packet_size` is the size of the packet.
    /// -`timestamp_delta` (output) is the computed send time delta.
    /// -`arrival_time_delta` (output) is the computed arrival-time delta.
    /// -`packet_size_delta` (output) is the computed size delta.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_deltas(
        &mut self,
        send_time: Timestamp,
        arrival_time: Timestamp,
        system_time: Timestamp,
        packet_size: usize,
        send_time_delta: &mut TimeDelta,
        arrival_time_delta: &mut TimeDelta,
        packet_size_delta: &mut i32,
    ) -> bool {
        let mut calculated_deltas = false;
        if self.current_timestamp_group.is_first_packet() {
            // We don't have enough data to update the filter, so we store it until we
            // have two frames of data to process.
            self.current_timestamp_group.send_time = send_time;
            self.current_timestamp_group.first_send_time = send_time;
            self.current_timestamp_group.first_arrival = arrival_time;
        } else if self.current_timestamp_group.first_send_time > send_time {
            // Reordered packet.
            return false;
        } else if self.new_timestamp_group(arrival_time, send_time) {
            // First packet of a later send burst, the previous packets sample is ready.
            if self.prev_timestamp_group.complete_time.is_finite() {
                *send_time_delta =
                    self.current_timestamp_group.send_time - self.prev_timestamp_group.send_time;
                *arrival_time_delta = self.current_timestamp_group.complete_time
                    - self.prev_timestamp_group.complete_time;
                let system_time_delta = self.current_timestamp_group.last_system_time
                    - self.prev_timestamp_group.last_system_time;
                if *arrival_time_delta - system_time_delta >= ARRIVAL_TIME_OFFSET_THRESHOLD {
                    log::warn!(
                        "The arrival time clock offset has changed (diff = {} ms), resetting.",
                        arrival_time_delta.ms() - system_time_delta.ms()
                    );
                    self.reset();
                    return false;
                }
                if *arrival_time_delta < TimeDelta::zero() {
                    // The group of packets has been reordered since receiving its local
                    // arrival timestamp.
                    self.num_consecutive_reordered_packets += 1;
                    if self.num_consecutive_reordered_packets >= REORDERED_RESET_THRESHOLD {
                        log::warn!(
                            "Packets between send burst arrived out of order, resetting:\"
                            arrival_time_delta_ms={}, send_time_delta_ms={}",
                            arrival_time_delta.ms(),
                            send_time_delta.ms()
                        );
                        self.reset();
                    }
                    return false;
                } else {
                    self.num_consecutive_reordered_packets = 0;
                }
                *packet_size_delta =
                    (self.current_timestamp_group.size - self.prev_timestamp_group.size) as i32;
                calculated_deltas = true;
            }
            self.prev_timestamp_group = self.current_timestamp_group.clone();
            // The new timestamp is now the current frame.
            self.current_timestamp_group.first_send_time = send_time;
            self.current_timestamp_group.send_time = send_time;
            self.current_timestamp_group.first_arrival = arrival_time;
            self.current_timestamp_group.size = 0;
        } else {
            self.current_timestamp_group.send_time =
                std::cmp::max(self.current_timestamp_group.send_time, send_time);
        }
        // Accumulate the frame size.
        self.current_timestamp_group.size += packet_size;
        self.current_timestamp_group.complete_time = arrival_time;
        self.current_timestamp_group.last_system_time = system_time;
        calculated_deltas
    }

    /// Returns true if the last packet was the end of the current batch and the packet with
    /// `send_time` is the first of a new batch.
    ///
    /// Assumes that `timestamp` is not reordered compared to `current_timestamp_group_`.
    fn new_timestamp_group(&self, arrival_time: Timestamp, send_time: Timestamp) -> bool {
        if self.current_timestamp_group.is_first_packet()
            || self.belongs_to_burst(arrival_time, send_time)
        {
            false
        } else {
            send_time - self.current_timestamp_group.first_send_time > self.send_time_group_length
        }
    }

    fn belongs_to_burst(&self, arrival_time: Timestamp, send_time: Timestamp) -> bool {
        debug_assert!(self.current_timestamp_group.complete_time.is_finite());
        let arrival_time_delta = arrival_time - self.current_timestamp_group.complete_time;
        let send_time_delta = send_time - self.current_timestamp_group.send_time;
        if send_time_delta.is_zero() {
            return true;
        }
        let propagation_delta = arrival_time_delta - send_time_delta;
        if propagation_delta < TimeDelta::zero()
            && arrival_time_delta <= BURST_DELTA_THRESHOLD
            && arrival_time - self.current_timestamp_group.first_arrival < MAX_BURST_DURATION
        {
            return true;
        }
        false
    }

    fn reset(&mut self) {
        self.num_consecutive_reordered_packets = 0;
        self.current_timestamp_group = SendTimeGroup::new();
        self.prev_timestamp_group = SendTimeGroup::new();
    }
}

#[derive(Debug, Clone)]
pub struct SendTimeGroup {
    size: usize,
    first_send_time: Timestamp,
    send_time: Timestamp,
    first_arrival: Timestamp,
    complete_time: Timestamp,
    last_system_time: Timestamp,
}

impl SendTimeGroup {
    pub fn new() -> Self {
        Self {
            size: 0,
            first_send_time: Timestamp::minus_infinity(),
            send_time: Timestamp::minus_infinity(),
            first_arrival: Timestamp::minus_infinity(),
            complete_time: Timestamp::minus_infinity(),
            last_system_time: Timestamp::minus_infinity(),
        }
    }

    pub fn is_first_packet(&self) -> bool {
        self.complete_time.is_infinite()
    }
}