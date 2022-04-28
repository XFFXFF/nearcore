#![allow(dead_code)]
use once_cell::sync::Lazy;

// TODO: consider wrapping these types to prevent interaction with
// real time in tests.
pub type Instant = std::time::Instant;
pub type Utc = chrono::DateTime<chrono::Utc>;
pub type Duration = chrono::Duration;

// Instant doesn't have a deterministic contructor,
// however since Instant is not convertible to an unix timestamp,
// we can snapshot Instant::now() at the process startup
// and treat it as a constant. All observable effects will be then
// deterministic.
static FAKE_CLOCK_MONO_START: Lazy<Instant> = Lazy::new(Instant::now);

// An arbitrary non-trivial deterministic Utc timestamp.
static FAKE_CLOCK_UTC_START: Lazy<Utc> =
    Lazy::new(|| Utc::from(std::time::SystemTime::UNIX_EPOCH) + Duration::seconds(89108233));

enum ClockInner<'a> {
    Real,
    Fake(&'a FakeClock),
}

pub struct Clock<'a>(ClockInner<'a>);

impl<'a> Clock<'a> {
    pub fn real() -> Clock<'static> {
        Clock(ClockInner::Real)
    }
    pub fn mono_now(&self) -> Instant {
        match self.0 {
            ClockInner::Real => Instant::now(),
            ClockInner::Fake(fake) => fake.mono_now(),
        }
    }
    pub fn utc_now(&self) -> Utc {
        match self.0 {
            ClockInner::Real => chrono::Utc::now(),
            ClockInner::Fake(fake) => fake.utc_now(),
        }
    }
}

pub struct FakeClock {
    mono: Instant,
    utc: Utc,
}

impl FakeClock {
    pub fn new(utc: Utc) -> Self {
        Self { utc, mono: *FAKE_CLOCK_MONO_START }
    }
    pub fn mono_now(&self) -> Instant {
        self.mono
    }
    pub fn utc_now(&self) -> Utc {
        self.utc
    }
    pub fn clock(&self) -> Clock<'_> {
        Clock(ClockInner::Fake(self))
    }
    pub fn advance(&mut self, d: Duration) {
        assert!(d >= Duration::zero());
        self.mono += d.to_std().unwrap();
        self.utc = self.utc + d;
    }
    pub fn set_utc(&mut self, utc: Utc) {
        self.utc = utc;
    }
}

impl Default for FakeClock {
    fn default() -> FakeClock {
        Self::new(*FAKE_CLOCK_UTC_START)
    }
}

pub fn make_rng(seed: u64) -> rand_pcg::Pcg32 {
    rand_pcg::Pcg32::new(seed, 0xa02bdbf7bb3c0a7)
}
