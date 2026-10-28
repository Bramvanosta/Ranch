//! The machine's clock.

use chrono::{DateTime, Utc};

use crate::application::ports::Clock;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
