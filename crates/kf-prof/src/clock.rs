//! Pure integer conversion of a calibrated counter to host monotonic nanoseconds.

/// An invalid calibration or unrepresentable timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockError {
    /// Frequency or calibration interval is zero.
    Invalid,
    /// Counter preceded its calibration anchor.
    Backwards,
    /// Checked conversion exceeded integer bounds.
    Overflow,
}

/// Immutable calibration; frequency is `frequency_num / frequency_den` Hz.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Calibration {
    /// Counter value at the calibration midpoint.
    counter_anchor: u64,
    /// Corresponding CLOCK_MONOTONIC nanoseconds.
    monotonic_anchor: u64,
    /// Frequency numerator, in Hz.
    frequency_num: u64,
    /// Positive frequency denominator.
    frequency_den: u64,
    /// Measured bracketing uncertainty, nanoseconds.
    error_ns: u64,
}

impl Calibration {
    /// Validate a rational frequency. Does not read a clock.
    pub fn new(
        counter_anchor: u64,
        monotonic_anchor: u64,
        frequency_num: u64,
        frequency_den: u64,
        error_ns: u64,
    ) -> Result<Self, ClockError> {
        if frequency_num == 0 || frequency_den == 0 {
            return Err(ClockError::Invalid);
        }
        Ok(Self {
            counter_anchor,
            monotonic_anchor,
            frequency_num,
            frequency_den,
            error_ns,
        })
    }

    /// Estimate rational frequency from two strictly increasing paired readings.
    pub fn from_pair(
        c0: u64,
        ns0: u64,
        c1: u64,
        ns1: u64,
        error_ns: u64,
    ) -> Result<Self, ClockError> {
        let dc = c1
            .checked_sub(c0)
            .filter(|&n| n > 0)
            .ok_or(ClockError::Invalid)?;
        let dt = ns1
            .checked_sub(ns0)
            .filter(|&n| n > 0)
            .ok_or(ClockError::Invalid)?;
        // Reduce before multiplication so ordinary long calibration windows fit.
        let g = gcd(dc, dt);
        let dc = dc / g;
        let dt = dt / g;
        let g = gcd(1_000_000_000, dt);
        let numerator = dc
            .checked_mul(1_000_000_000 / g)
            .ok_or(ClockError::Overflow)?;
        Self::new(c0, ns0, numerator, dt / g, error_ns)
    }

    /// Immutable wire fields: counter/monotonic anchors, frequency numerator,
    /// frequency denominator, and measured bracket uncertainty.
    pub fn parts(self) -> [u64; 5] {
        [
            self.counter_anchor,
            self.monotonic_anchor,
            self.frequency_num,
            self.frequency_den,
            self.error_ns,
        ]
    }

    /// Convert without floating point; invalid/backward/overflow is explicit.
    pub fn to_ns(self, counter: u64) -> Result<u64, ClockError> {
        let delta = counter
            .checked_sub(self.counter_anchor)
            .ok_or(ClockError::Backwards)?;
        let scaled = u128::from(delta)
            .checked_mul(1_000_000_000)
            .and_then(|n| n.checked_mul(u128::from(self.frequency_den)))
            .ok_or(ClockError::Overflow)?;
        let ns = scaled / u128::from(self.frequency_num);
        let ns = u64::try_from(ns).map_err(|_| ClockError::Overflow)?;
        self.monotonic_anchor
            .checked_add(ns)
            .ok_or(ClockError::Overflow)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}
