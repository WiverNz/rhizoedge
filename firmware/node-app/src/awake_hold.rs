//! The awake hold that gates sleep during a watering cycle (M9-021).
//!
//! # A guard, not a flag
//!
//! A flag that must be cleared on every path is a flag that will be left set on
//! some path, and a stuck hold means a battery device that never sleeps again —
//! a silent, expensive failure that looks like nothing at all until the battery
//! is flat. A guard whose `Drop` releases the hold gets every error path for
//! free, including the `?` that returns three frames up.
//!
//! # What the hold does and does not gate
//!
//! It gates **sleep**. It does not gate the run guard:
//! `FIRMWARE_MAX_RUN_SECONDS` still de-energises the pump on its independent
//! timer regardless of anything the wake cycle believes (F-090-37, F-090-58).
//! The two mechanisms are unrelated and must stay that way — one bounds how
//! long water can flow, the other bounds when the device may stop paying
//! attention.

use std::cell::Cell;
use std::rc::Rc;

/// A shared count of outstanding holds.
///
/// Counted rather than boolean so a dose whose result is still unacknowledged
/// when the next one starts cannot have its hold released by the first one's
/// guard.
#[derive(Clone, Debug, Default)]
pub struct HoldCount(Rc<Cell<u32>>);

impl HoldCount {
    /// A count with nothing held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many holds are outstanding.
    #[must_use]
    pub fn get(&self) -> u32 {
        self.0.get()
    }

    /// Whether anything is held.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.0.get() > 0
    }

    /// Acquires a hold. It is released when the returned guard is dropped.
    #[must_use]
    pub fn acquire(&self) -> AwakeHold {
        self.0.set(self.0.get().saturating_add(1));
        AwakeHold(self.0.clone())
    }
}

/// An outstanding awake hold. Dropping it releases the hold.
#[derive(Debug)]
pub struct AwakeHold(Rc<Cell<u32>>);

impl Drop for AwakeHold {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_released_when_its_guard_is_dropped() {
        let count = HoldCount::new();
        assert!(!count.is_held());
        {
            let _hold = count.acquire();
            assert!(count.is_held());
        }
        assert!(!count.is_held());
    }

    #[test]
    fn nested_holds_release_independently() {
        let count = HoldCount::new();
        let first = count.acquire();
        let second = count.acquire();
        assert_eq!(count.get(), 2);
        drop(second);
        assert!(count.is_held());
        drop(first);
        assert!(!count.is_held());
    }

    /// The reason it is a guard: an early return from an error path releases
    /// the hold without the author having remembered to.
    #[test]
    fn the_hold_is_released_on_an_error_return() {
        fn dose(count: &HoldCount, fail: bool) -> Result<(), &'static str> {
            let _hold = count.acquire();
            if fail {
                return Err("pump refused");
            }
            Ok(())
        }
        let count = HoldCount::new();
        assert!(dose(&count, true).is_err());
        assert!(!count.is_held(), "an error path still released the hold");
        assert!(dose(&count, false).is_ok());
        assert!(!count.is_held());
    }
}
