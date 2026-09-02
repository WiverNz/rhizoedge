//! Battery voltage, where the board has a divider (M9-021).
//!
//! # Absent, never zero
//!
//! A board with no divider constructs no `BatterySensor`, and the sampling code
//! holds an `Option`. That is what makes "battery fields are omitted on
//! hardware that cannot measure them" a fact about the type rather than a rule
//! somebody has to remember (ADR-018 §7).
//!
//! # Power is never a safety input
//!
//! Nothing here reaches `IrrigationInputs` or any argument to
//! `validate_water_command`. A low battery raises an alert and refuses nothing.
//! `../node-app/tests/board_isolation.rs` checks that structurally.
//!
//! # The divider ratio is not calibrated here
//!
//! `divider_ratio` is supplied by the board profile and is a nominal value from
//! the resistor pair. Calibrating it against a reference meter is M10-012, on a
//! board, with a meter — so the reading this produces is honest about being a
//! nominal conversion rather than a measured one.

use rhizo_node_app::ports::{BatterySensor, SensorError};

/// A battery divider on one ADC channel.
///
/// Generic over the reader so the ADC construction — which is board-specific —
/// stays in `src/board/`, and so this file names no channel and no attenuation.
#[allow(
    dead_code,
    reason = "the trait and its adapter are M9-021's deliverable; the board profile \n              that constructs one needs a divider, which the DEVKITM-1 reference \n              board does not have. M10-012 measures the first board that does."
)]
pub struct DividerBattery<R> {
    read_mv: R,
    divider_ratio: f32,
}

// M9-021's deliverable is the trait, its fake, and this adapter. Constructing
// one needs a board with a divider, and the DEVKITM-1 reference board has none
// — battery fields are **absent** on hardware that cannot measure them, which
// is exactly the state this represents. M10-012 measures the first board that
// can, and the profile that names its ADC channel constructs this then.
#[allow(
    dead_code,
    reason = "constructed by the first board profile with a divider"
)]
impl<R> DividerBattery<R>
where
    R: FnMut() -> Result<u32, esp_idf_sys::EspError>,
{
    /// Wraps an ADC reader and the board's nominal divider ratio.
    ///
    /// `divider_ratio` is `(r_top + r_bottom) / r_bottom`: what the pin sees
    /// multiplied by this is what the pack is at.
    #[must_use]
    pub const fn new(read_mv: R, divider_ratio: f32) -> Self {
        Self {
            read_mv,
            divider_ratio,
        }
    }
}

impl<R> BatterySensor for DividerBattery<R>
where
    R: FnMut() -> Result<u32, esp_idf_sys::EspError>,
{
    fn read_mv(&mut self) -> Result<u32, SensorError> {
        let at_pin = (self.read_mv)().map_err(|_| SensorError::ReadFailed)?;
        let scaled = f64::from(at_pin) * f64::from(self.divider_ratio);
        if !scaled.is_finite() || scaled < 0.0 {
            // A non-finite conversion is an unusable reading, and an unusable
            // reading is an error rather than a plausible number.
            return Err(SensorError::ReadFailed);
        }
        Ok(scaled.min(f64::from(u32::MAX)) as u32)
    }
}
