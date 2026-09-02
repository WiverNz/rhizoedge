//! A switched peripheral supply (M9-020).
//!
//! The pin and its active level arrive from the board profile: a load switch
//! that is active-low on one board and active-high on another is a board fact,
//! and getting it backwards powers a rail through a whole sleep.

use esp_idf_hal::gpio::{Output, PinDriver};
use rhizo_node_app::ports::PowerRail;

/// A rail driven by one GPIO at a board-supplied polarity.
pub struct GpioRail<'d> {
    pin: PinDriver<'d, Output>,
    active_high: bool,
    gpio_number: u8,
    enabled: bool,
}

impl<'d> GpioRail<'d> {
    /// Wraps a pin, **driving it off before returning**.
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
            "rail on GPIO {gpio_number} (active {}), off",
            if active_high { "high" } else { "low" }
        );
        Ok(Self {
            pin,
            active_high,
            gpio_number,
            enabled: false,
        })
    }
}

impl PowerRail for GpioRail<'_> {
    fn enable(&mut self) {
        let driven = if self.active_high {
            self.pin.set_high()
        } else {
            self.pin.set_low()
        };
        // `is_enabled` reports what the pin actually took, not what was asked
        // for. A rail that failed to come up must not look powered, because the
        // warm-up timer would then start counting for a sensor with no supply.
        if driven.is_err() {
            log::error!("rail on GPIO {} would not come up", self.gpio_number);
        }
        self.enabled = driven.is_ok();
    }

    fn disable(&mut self) {
        let driven = if self.active_high {
            self.pin.set_low()
        } else {
            self.pin.set_high()
        };
        if driven.is_ok() {
            self.enabled = false;
        } else {
            // A rail that will not go off is a rail powered through a whole
            // sleep, so it is an error and not a warning.
            log::error!("rail on GPIO {} would not go off", self.gpio_number);
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}
