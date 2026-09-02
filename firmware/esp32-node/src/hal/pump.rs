//! The pump output (M9-007; the calibrated driver is M11-001).
//!
//! The pin and its active level arrive from the board profile. This file names
//! neither, and the `gpio_number` it carries is a diagnostic string for logs,
//! not a source of truth about wiring.

use esp_idf_hal::gpio::{Output, PinDriver};
use rhizo_node_app::ports::{Pump, PumpError};

/// A pump driven by one GPIO at a board-supplied polarity.
pub struct GpioPump<'d> {
    pin: PinDriver<'d, Output>,
    active_high: bool,
    gpio_number: u8,
    faulted: bool,
}

impl<'d> GpioPump<'d> {
    /// Wraps a pin, **driving it inactive before returning**.
    ///
    /// There is deliberately no way to construct a `GpioPump` that has not
    /// already been de-energised: a constructed pump in an unknown state is a
    /// window, and this is the one place windows are not acceptable.
    ///
    /// # Errors
    ///
    /// If the pin cannot be driven.
    pub fn new(
        mut pin: PinDriver<'d, Output>,
        active_high: bool,
        gpio_number: u8,
    ) -> Result<Self, esp_idf_sys::EspError> {
        if active_high {
            pin.set_low()?;
        } else {
            pin.set_high()?;
        }
        log::info!(
            "pump on GPIO {gpio_number} (active {}); external pull-down required",
            if active_high { "high" } else { "low" }
        );
        Ok(Self {
            pin,
            active_high,
            gpio_number,
            faulted: false,
        })
    }

    fn energise(&mut self) -> Result<(), esp_idf_sys::EspError> {
        if self.active_high {
            self.pin.set_high()
        } else {
            self.pin.set_low()
        }
    }

    fn de_energise(&mut self) -> Result<(), esp_idf_sys::EspError> {
        if self.active_high {
            self.pin.set_low()
        } else {
            self.pin.set_high()
        }
    }
}

impl Pump for GpioPump<'_> {
    /// Runs for `ms`, then de-energises.
    ///
    /// The delay here is a blocking wait on the calling task, and it is the
    /// *inner* bound only. `FIRMWARE_MAX_RUN_SECONDS` is enforced independently
    /// by the run guard, which does not trust this function to return
    /// (F-090-37, F-090-58). A driver that latches a fault refuses outright
    /// rather than trying and failing halfway.
    fn run_for(&mut self, ms: u32) -> Result<(), PumpError> {
        if self.faulted {
            return Err(PumpError::Faulted);
        }
        if self.energise().is_err() {
            self.faulted = true;
            let _ = self.de_energise();
            return Err(PumpError::DriveFailed);
        }
        esp_idf_hal::delay::FreeRtos::delay_ms(ms);
        if self.de_energise().is_err() {
            // A pin that will not go low is the worst outcome this driver has,
            // so it latches: every subsequent command is refused rather than
            // adding water to a pump that may already be running.
            self.faulted = true;
            return Err(PumpError::DriveFailed);
        }
        Ok(())
    }

    fn off(&mut self) {
        if self.de_energise().is_err() {
            // The worst outcome this driver has: a pin that will not go low.
            // Named with its GPIO because the next thing anyone does is put a
            // meter on it.
            log::error!("pump on GPIO {} would not de-energise", self.gpio_number);
            self.faulted = true;
        }
    }

    fn is_faulted(&self) -> bool {
        self.faulted
    }
}
