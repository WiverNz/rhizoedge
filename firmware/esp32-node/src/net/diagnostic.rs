//! Secret-free diagnostic record; no decisions or driver configuration.
use core::fmt;

pub struct Record {
    pub at_ms: u64,
    pub attempt: u32,
    pub event: &'static str,
    pub reason: Option<u16>,
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NET_DIAG at_ms={} attempt={} event={}",
            self.at_ms, self.attempt, self.event
        )?;
        if let Some(reason) = self.reason {
            write!(f, " reason={reason}")?;
        }
        Ok(())
    }
}
