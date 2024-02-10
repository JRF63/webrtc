const PLUS_INFINITY_VAL: i64 = i64::MAX;
const MINUS_INFINITY_VAL: i64 = i64::MIN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    value: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeDelta {
    value: i64,
}

macro_rules! microsecond_resolution {
    ($type_name:ty) => {
        impl $type_name {
            pub const fn from_minutes(value: i64) -> Self {
                Self::from_seconds(60 * value)
            }

            pub const fn from_seconds(value: i64) -> Self {
                Self::from_micros(1_000_000 * value)
            }

            pub const fn from_millis(value: i64) -> Self {
                Self::from_micros(1_000 * value)
            }

            pub const fn from_micros(value: i64) -> Self {
                Self { value }
            }

            pub const fn from_nanos(value: i64) -> Self {
                Self::from_micros(value / 1_000)
            }

            pub const fn zero() -> Self {
                Self { value: 0 }
            }

            pub const fn plus_infinity() -> Self {
                Self::from_micros(PLUS_INFINITY_VAL)
            }

            pub const fn minus_infinity() -> Self {
                Self::from_micros(MINUS_INFINITY_VAL)
            }

            pub const fn seconds(&self) -> i64 {
                self.us() / 1_000_000
            }

            pub const fn ms(&self) -> i64 {
                self.us() / 1_000
            }

            pub const fn us(&self) -> i64 {
                self.value
            }

            pub const fn ns(&self) -> i64 {
                1000 * self.us()
            }

            pub const fn is_zero(&self) -> bool {
                self.value == 0
            }

            pub const fn is_plus_infinity(&self) -> bool {
                self.us() == PLUS_INFINITY_VAL
            }

            pub const fn is_minus_infinity(&self) -> bool {
                self.us() == MINUS_INFINITY_VAL
            }

            pub const fn is_infinite(&self) -> bool {
                self.is_plus_infinity() || self.is_minus_infinity()
            }

            pub const fn is_finite(&self) -> bool {
                !self.is_infinite()
            }
        }
    };
}

microsecond_resolution!(Timestamp);
microsecond_resolution!(TimeDelta);

impl std::ops::Add<TimeDelta> for Timestamp {
    type Output = Self;

    fn add(self, rhs: TimeDelta) -> Self::Output {
        if self.is_plus_infinity() || rhs.is_plus_infinity() {
            Self::plus_infinity()
        } else if self.is_minus_infinity() || rhs.is_minus_infinity() {
            Self::minus_infinity()
        } else {
            Self::from_micros(self.us() + rhs.us())
        }
    }
}

impl std::ops::Sub<TimeDelta> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: TimeDelta) -> Self::Output {
        if self.is_plus_infinity() || rhs.is_minus_infinity() {
            Self::plus_infinity()
        } else if self.is_minus_infinity() || rhs.is_plus_infinity() {
            Self::minus_infinity()
        } else {
            Self::from_micros(self.us() - rhs.us())
        }
    }
}

impl std::ops::AddAssign<TimeDelta> for Timestamp {
    fn add_assign(&mut self, rhs: TimeDelta) {
        *self = *self + rhs
    }
}

impl std::ops::SubAssign<TimeDelta> for Timestamp {
    fn sub_assign(&mut self, rhs: TimeDelta) {
        *self = *self - rhs
    }
}

impl std::ops::Sub<Timestamp> for Timestamp {
    type Output = TimeDelta;

    fn sub(self, rhs: Timestamp) -> Self::Output {
        Self::Output {
            value: self.value - rhs.value,
        }
    }
}

impl std::ops::Add for TimeDelta {
    type Output = Self;

    fn add(self, rhs: TimeDelta) -> Self::Output {
        Self {
            value: self.value + rhs.value,
        }
    }
}

impl std::ops::Sub for TimeDelta {
    type Output = Self;

    fn sub(self, rhs: TimeDelta) -> Self::Output {
        Self {
            value: self.value - rhs.value,
        }
    }
}

impl std::ops::Mul<f64> for TimeDelta {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            value: (self.value as f64 * rhs) as i64,
        }
    }
}

// TODO: Tests
