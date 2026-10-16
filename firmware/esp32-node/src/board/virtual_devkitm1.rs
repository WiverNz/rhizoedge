//! Explicit no-pin bench adapter for the real MCU and radio.
use crate::bench::VirtualPump;
use crate::board::{Board, BoardProfile};
use esp_idf_hal::peripherals::Peripherals;
use rhizo_node_app::fakes::FakeRail;

/// Virtual-actuator bench; never constructs a physical output.
pub struct Devkitm1;
impl BoardProfile for Devkitm1 {
    type Pump = VirtualPump;
    type Rail = FakeRail;
    const PROFILE: &'static str = "devkitm1-VIRTUAL-ACTUATOR-BENCH";
    fn take(peripherals: Peripherals) -> Result<Board<Self>, esp_idf_sys::EspError> {
        Ok(Board {
            pump: VirtualPump::default(),
            sensor_rail: None,
            rs485_rail: None,
            modem: peripherals.modem,
        })
    }
}
