//! Espressif ESP32-C3-DEVKITM-1-N4X — the initial development and reference
//! board (ADR-007, amended 2026-08-28; F-090-43).
//!
//! Chosen for bring-up convenience rather than for deployment: a full pin
//! header for breadboarding, an on-board USB-to-UART bridge for serial
//! provisioning and `espflash --monitor`, exposed strapping pins, and a form
//! factor that tolerates a multimeter probe on the pump driver input — which is
//! exactly what HIL-1 requires.
//!
//! # The pin plan (2026-10-23)
//!
//! This profile **constructs** only GPIO5, 6 and 7 today. The rest of the table
//! is the permanent allocation the later adapters must use, so that every
//! peripheral below can be wired at once with no replugging and no second
//! profile. Rationale, datasheet references and wiring are in
//! `docs/hardware/home-node-hardware-guide.md` §DEVKITM-1 pin allocation.
//!
//! | GPIO | Use | Dir | State from reset until configured | External part |
//! |---|---|---|---|---|
//! | 5 | **pump driver input** (constructed) | out | input, no pull, no glitch | **10 kΩ pull-down** |
//! | 6 | sensor rail enable (constructed) | out | input **with weak pull-up** (JTAG pad) | 10 kΩ pull-down |
//! | 7 | RS485 rail enable (constructed) | out | input, no pull | 10 kΩ pull-down |
//! | 10 | RS485 DE + /RE, tied | out | input, no pull | 10 kΩ pull-down |
//! | 18 | UART1 TX → transceiver DI | out | 50 µs high glitch | — |
//! | 19 | UART1 RX ← transceiver RO | in | USB D+ pull-up | — |
//! | 3 | I2C SDA (INA260; bench OLED) | od | input, no pull | 4.7 kΩ to 3V3 |
//! | 2 | I2C SCL — strapping pin | od | floating | 4.7 kΩ to 3V3, **required** |
//! | 0 | leak input | in | floating | pull-up per sensor |
//! | 1 | tank input | in | floating | pull-up per sensor |
//! | 4 | analogue soil probe, ADC1 ch4 | in | input, no pull | — |
//! | 8 | on-board RGB LED, strapping pin | out | — | fitted on board |
//! | 9 | BOOT button, strapping pin | in | weak pull-up | fitted on board |
//! | 20, 21 | UART0 console via CP2102N | — | — | fitted on board |
//!
//! Hard rules this table encodes:
//!
//! - **The pump pin shares nothing.** GPIO5 has neither a reset pull-up nor a
//!   power-up glitch (ESP32-C3 datasheet v2.4, Tables 2-1 and 2-2), and its
//!   only analogue function is ADC2, which ESP-IDF refuses on the C3. GPIO18
//!   (50 µs high glitch), GPIO19 (USB pull-up) and GPIO6 (JTAG pull-up) must
//!   never drive a pump or any other active-high load whose accidental
//!   assertion matters.
//! - **SDA is never on a strapping pin**: a slave cut off mid-transfer can hold
//!   SDA low through a reset. SCL on GPIO2 is safe because only the master
//!   drives SCL, neither the INA260 nor the SH1106 stretches the clock, and the
//!   required pull-up holds GPIO2 at the level the datasheet recommends.
//! - **No pin is spare.** Further expansion goes on the I2C bus, never by
//!   doubling up a pin. GPIO 11–17 are the in-package flash and are not on the
//!   header; the micro-USB connector reaches GPIO18/19 only through unfitted
//!   R2/R3, so on this board native USB is unused and the console is UART0.
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
//! HIL-1 proves the wiring with transient capture on the driver input across
//! twenty resets, a watchdog reset, and ten mid-boot power cuts. A DMM alone
//! cannot exclude short pulses. **A board
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
