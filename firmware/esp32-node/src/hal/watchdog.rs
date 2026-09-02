//! The hardware watchdog (F-090-38).
//!
//! A watchdog reset leaves the pump off, and it does so for the same reason
//! every other reset does: `main` drives the pump inactive as its first
//! statement, and a watchdog reset is an ordinary reset. Nothing here has to
//! arrange that, which is the point — a watchdog whose correctness depended on
//! a special path would be one more path to get wrong.

use rhizo_node_app::ports::Watchdog;

/// The ESP-IDF task watchdog, subscribed for the current task.
#[derive(Clone, Copy, Debug, Default)]
pub struct TaskWatchdog {
    subscribed: bool,
}

impl TaskWatchdog {
    /// Subscribes the current task to the task watchdog.
    ///
    /// The watchdog itself is initialised from `sdkconfig.defaults`
    /// (`CONFIG_ESP_TASK_WDT_INIT`), so this only adds the main task to it.
    ///
    /// # Errors
    ///
    /// If the task cannot be added.
    pub fn subscribe() -> Result<Self, esp_idf_sys::EspError> {
        // SAFETY: `esp_task_wdt_add(null)` adds the *current* task, which is
        // the documented use of a null handle.
        let code = unsafe { esp_idf_sys::esp_task_wdt_add(core::ptr::null_mut()) };
        if code != esp_idf_sys::ESP_OK {
            log::warn!("task watchdog subscribe failed: {code}");
            return Ok(Self { subscribed: false });
        }
        Ok(Self { subscribed: true })
    }

    /// Whether the current task is actually being watched.
    #[must_use]
    pub const fn is_subscribed(&self) -> bool {
        self.subscribed
    }
}

impl Watchdog for TaskWatchdog {
    fn feed(&mut self) {
        if !self.subscribed {
            return;
        }
        // SAFETY: resets the current task's watchdog counter; no preconditions.
        unsafe {
            esp_idf_sys::esp_task_wdt_reset();
        }
    }
}
