use std::collections::HashMap;
#[derive(Default)]
pub struct Limiter {
    windows: HashMap<String, (i64, u8)>,
}
impl Limiter {
    pub fn allow(&mut self, device: &str, now: i64) -> bool {
        let e = self.windows.entry(device.to_owned()).or_insert((now, 0));
        if now - e.0 >= 60_000 {
            *e = (now, 0)
        }
        if e.1 >= 10 {
            return false;
        }
        e.1 += 1;
        true
    }
}
#[cfg(test)]
#[allow(
    clippy::module_inception,
    reason = "keeps the issue's literal quarantine:: verification filter"
)]
mod quarantine {
    use super::*;
    #[test]
    fn ten_per_minute_per_device() {
        let mut l = Limiter::default();
        for _ in 0..10 {
            assert!(l.allow("node-01", 0))
        }
        assert!(!l.allow("node-01", 0));
        assert!(l.allow("node-01", 60_000));
        assert!(l.allow("node-02", 0));
    }
}
