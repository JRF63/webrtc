// `Random` is technically unnecessary and only helps in having the exact same test cases as the
// original implementation.
pub struct Random {
    state: u64,
}

impl Random {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn rand(&mut self, t: u32) -> u32 {
        let x = self.next_output() as u32;
        let mut result: u64 = x as u64 * (t as u64 + 1);
        result >>= 32;
        result as u32
    }

    pub fn gaussian(&mut self, mean: f64, standard_deviation: f64) -> f64 {
        let u1 = self.next_output() as f64 / u64::MAX as f64;
        let u2 = self.next_output() as f64 / u64::MAX as f64;
        return mean
            + standard_deviation
                * f64::sqrt(-2.0 * f64::ln(u1))
                * f64::cos(2.0 * std::f64::consts::PI * u2);
    }

    fn next_output(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.overflowing_mul(2685821657736338717).0
    }
}
