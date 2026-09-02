//! Espressif ESP32-C3-DEVKITM-1-N4X — the initial development and reference
//! board (ADR-007, amended 2026-08-28; F-090-43).
//!
//! Chosen for bring-up convenience rather than for deployment: a full pin
//! header for breadboarding, an on-board USB-to-UART bridge for serial
//! provisioning and `espflash --monitor`, exposed strapping pins, and a form
//! factor that tolerates a multimeter probe on the pump driver input — which is
//! exactly what HIL-1 requires.
//!
//! # The pin map, and why these pins
//!
//! ESP32-C3 GPIO 2, 8 and 9 are strapping pins and are avoided: their level at
//! reset selects the boot mode, so driving one is a way to make a board
//! unflashable. GPIO 11–17 serve the SPI flash on an N4X module. GPIO 18 and 19
//! are the USB Serial/JTAG pair and are left alone so `espflash --monitor`
//! keeps working. What remains — 0, 1, 3, 4, 5, 6, 7, 10 — is what this profile
//! uses.
//!
//! # THE HARDWARE REQUIREMENT SOFTWARE CANNOT SATISFY
//!
//! **The pump driver input MUST have an external pull-down to ground**
//! (10 kΩ is ample), so that an un-driven pin is electrically pump-off
//! (F-090-31, SAFETY-011).
//!
//! This is not a preference. Between reset and the first statement of `main`
//! the pin floats: the ROM bootloader runs, the second-stage bootloader runs,
//! and the ESP-IDF startup code runs, and none of them knows this pin drives a
//! pump. Firmware cannot cover that window from inside it. If the driver is
//! wired so that a floating input turns the pump *on*, no amount of correct
//! firmware helps, and the failure is a plant sitting in a puddle after a
//! brownout.
//!
//! HIL-1 is what proves the wiring: a multimeter on the driver input across
//! twenty resets, a watchdog reset, and ten mid-boot power cuts. **A board
//! profile that cannot satisfy this is not a supported board**, and every
//! future profile carries the same note against its own pump pin.
//!
//! The pump GPIO is also configured with its internal pull-down enabled. That
//! is belt-and-braces and explicitly **not** a substitute: the internal
//! pull-down is only active once the GPIO matrix has been configured, which is
//! after the window that matters.

use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::peripherals::Peripherals;

use crate::board::{Board, BoardProfile};
use crate::hal::pump::GpioPump;
use crate::hal::rail::GpioRail;

/// The pump control output.
///
/// **Requires an external pull-down to ground.** See the module documentation;
/// this is the one constraint firmware cannot enforce.
const PUMP_GPIO: u8 = 5;
/// Active level of the pump driver input.
const PUMP_ACTIVE_HIGH: bool = true;

/// The sensor supply load-switch enable.
const SENSOR_RAIL_GPIO: u8 = 6;
/// Active level of the sensor load switch.
const SENSOR_RAIL_ACTIVE_HIGH: bool = true;

/// The RS485 transceiver supply load-switch enable.
const RS485_RAIL_GPIO: u8 = 7;
/// Active level of the RS485 load switch.
///
/// Getting this backwards powers a rail through a whole sleep, which is why it
/// is a board fact and lives here rather than in the sampling code.
const RS485_RAIL_ACTIVE_HIGH: bool = true;

/// The reference development board.
pub struct Devkitm1;

impl BoardProfile for Devkitm1 {
    type Pump = GpioPump<'static>;
    type Rail = GpioRail<'static>;

    const PROFILE: &'static str = "devkitm1";

    fn take(peripherals: Peripherals) -> Result<Board<Self>, esp_idf_sys::EspError> {
        let pins = peripherals.pins;
        let modem = peripherals.modem;

        // The pump first, and de-energised by construction: `GpioPump::new`
        // drives the inactive level before it returns, so there is no window in
        // which a constructed pump is in an unknown state.
        let pump = GpioPump::new(PinDriver::output(pins.gpio5)?, PUMP_ACTIVE_HIGH, PUMP_GPIO)?;

        let sensor_rail = GpioRail::new(
            PinDriver::output(pins.gpio6)?,
            SENSOR_RAIL_ACTIVE_HIGH,
            SENSOR_RAIL_GPIO,
        )?;
        let rs485_rail = GpioRail::new(
            PinDriver::output(pins.gpio7)?,
            RS485_RAIL_ACTIVE_HIGH,
            RS485_RAIL_GPIO,
        )?;

        Ok(Board {
            pump,
            sensor_rail: Some(sensor_rail),
            rs485_rail: Some(rs485_rail),
            modem,
        })
    }
}
