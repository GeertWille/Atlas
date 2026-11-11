use std::time::{SystemTime, UNIX_EPOCH};

use crate::application::ports::{Clock, IdGenerator};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
    }
}

pub struct UuidGenerator;

impl IdGenerator for UuidGenerator {
    fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
