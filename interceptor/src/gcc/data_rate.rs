use super::time::TimeDelta;

const PLUS_INFINITY_VAL: i64 = i64::MAX;
const MINUS_INFINITY_VAL: i64 = i64::MIN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataRate {
    value: i64, // TODO: Maybe use `f64` instead
}

impl DataRate {
    pub const fn from_bits_per_sec(value: i64) -> Self {
        Self { value }
    }

    pub const fn from_bytes_per_sec(value: i64) -> Self {
        Self { value: 8 * value }
    }

    pub const fn from_kilobits_per_sec(value: i64) -> Self {
        Self {
            value: 1000 * value,
        }
    }

    pub const fn infinity() -> Self {
        Self::plus_infinity()
    }

    pub const fn bps(&self) -> i64 {
        self.value
    }

    pub const fn bytes_per_sec(&self) -> i64 {
        self.value / 8
    }

    pub const fn kbps(&self) -> i64 {
        self.value / 1000
    }

    pub const fn zero() -> Self {
        Self { value: 0 }
    }

    pub const fn plus_infinity() -> Self {
        Self {
            value: PLUS_INFINITY_VAL,
        }
    }

    pub const fn minus_infinity() -> Self {
        Self {
            value: MINUS_INFINITY_VAL,
        }
    }

    pub const fn is_zero(&self) -> bool {
        self.value == 0
    }

    pub const fn is_plus_infinity(&self) -> bool {
        self.value == PLUS_INFINITY_VAL
    }

    pub const fn is_minus_infinity(&self) -> bool {
        self.value == MINUS_INFINITY_VAL
    }

    pub const fn is_infinite(&self) -> bool {
        self.is_plus_infinity() || self.is_minus_infinity()
    }

    pub const fn is_finite(&self) -> bool {
        !self.is_infinite()
    }
}

impl std::ops::Add for DataRate {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value + rhs.value,
        }
    }
}

impl std::ops::Sub for DataRate {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            value: self.value - rhs.value,
        }
    }
}

impl std::ops::SubAssign for DataRate {
    fn sub_assign(&mut self, rhs: Self) {
        self.value -= rhs.value
    }
}

impl std::ops::Mul<f64> for DataRate {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            value: (self.value as f64 * rhs) as i64,
        }
    }
}

impl std::ops::Mul<DataRate> for f64 {
    type Output = DataRate;

    fn mul(self, rhs: DataRate) -> Self::Output {
        rhs * self
    }
}

impl std::ops::Mul<TimeDelta> for DataRate {
    type Output = DataSize;

    fn mul(self, rhs: TimeDelta) -> Self::Output {
        let microbits = self.bps() * rhs.us();
        DataSize::from_bytes((microbits + 4000000) / 8000000)
    }
}

impl std::ops::Div<i64> for DataRate {
    type Output = Self;

    fn div(self, rhs: i64) -> Self::Output {
        Self {
            value: self.value / rhs,
        }
    }
}

impl std::ops::Div<f64> for DataRate {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        Self {
            value: (self.value as f64 / rhs) as i64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataSize {
    value: i64,
}

impl DataSize {
    pub const fn from_bytes(value: i64) -> Self {
        Self { value }
    }

    pub const fn microbits(&self) -> i64 {
        self.value * 8_000_000
    }

    pub const fn bytes(&self) -> i64 {
        self.value
    }

    pub const fn zero() -> Self {
        Self { value: 0 }
    }
}

impl std::ops::Div<f64> for DataSize {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        Self {
            value: (self.value as f64 / rhs) as i64,
        }
    }
}

impl std::ops::Div<DataRate> for DataSize {
    type Output = TimeDelta;

    fn div(self, rhs: DataRate) -> Self::Output {
        TimeDelta::from_micros(self.microbits() / rhs.bps())
    }
}

impl std::ops::Div<TimeDelta> for DataSize {
    type Output = DataRate;

    fn div(self, rhs: TimeDelta) -> Self::Output {
        DataRate::from_bits_per_sec(self.microbits() / rhs.us())
    }
}
