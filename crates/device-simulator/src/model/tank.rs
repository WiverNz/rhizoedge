//! The reservoir, and the leak sensor that sits under the tray.
//!
//! ```text
//! tank_percent -= delivered_ml / tank_capacity_ml * 100
//! leak_detected = injected only
//! ```
//!
//! The leak state is **never** produced by the model: a leak is a fault an
//! operator or a scenario injects, because the point of the leak input is to
//! test what the controller does when it is asserted, not to model plumbing.
//! It is nonetheless a tri-state — a sensor that cannot be read is `Unknown`,
//! and `Unknown` maps to a lockout, never to permission (SAFETY-012).

use rhizo_mqtt_contract::safety::LeakState;

use crate::rng::SplitMix64;

/// Tunable reservoir parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TankParams {
    /// Full capacity in millilitres.
    pub capacity_ml: f64,
    /// Gaussian noise sigma on the level reading, in percent.
    pub noise_sigma_percent: f64,
}

impl Default for TankParams {
    fn default() -> Self {
        Self {
            capacity_ml: 2_000.0,
            noise_sigma_percent: 0.5,
        }
    }
}

/// The reservoir.
#[derive(Clone, Debug)]
pub struct TankModel {
    params: TankParams,
    remaining_ml: f64,
    leak: LeakState,
}

impl TankModel {
    /// Creates a reservoir at a starting level, in percent.
    #[must_use]
    pub fn new(params: TankParams, initial_percent: f64) -> Self {
        let remaining_ml = params.capacity_ml * initial_percent.clamp(0.0, 100.0) / 100.0;
        Self {
            params,
            remaining_ml,
            leak: LeakState::Clear,
        }
    }

    /// The parameters in force.
    #[must_use]
    pub const fn params(&self) -> &TankParams {
        &self.params
    }

    /// Volume remaining.
    #[must_use]
    pub const fn remaining_ml(&self) -> f64 {
        self.remaining_ml
    }

    /// The true level as a percentage, before noise.
    #[must_use]
    pub fn true_percent(&self) -> f64 {
        if self.params.capacity_ml <= 0.0 {
            return 0.0;
        }
        (self.remaining_ml / self.params.capacity_ml * 100.0).clamp(0.0, 100.0)
    }

    /// The leak sensor's current state.
    #[must_use]
    pub const fn leak(&self) -> LeakState {
        self.leak
    }

    /// Sets the leak state. Injection is the only way it changes.
    pub const fn set_leak(&mut self, state: LeakState) {
        self.leak = state;
    }

    /// Draws water for a delivery, returning what was actually available.
    ///
    /// A pump cannot deliver water that is not there, and the level never goes
    /// negative — which is what makes "tank empty" a testable condition rather
    /// than an arithmetic curiosity.
    pub fn draw(&mut self, ml: f64) -> f64 {
        if !ml.is_finite() || ml <= 0.0 {
            return 0.0;
        }
        let drawn = ml.min(self.remaining_ml);
        self.remaining_ml -= drawn;
        drawn
    }

    /// Sets the level directly, for the control API and the `tank-empty` fault.
    pub fn set_percent(&mut self, percent: f64) {
        self.remaining_ml = self.params.capacity_ml * percent.clamp(0.0, 100.0) / 100.0;
    }

    /// What the level sensor reports.
    pub fn sample_percent(&self, rng: &mut SplitMix64, noise: bool) -> f64 {
        let sigma = if noise {
            self.params.noise_sigma_percent
        } else {
            0.0
        };
        (self.true_percent() + rng.gaussian(sigma)).clamp(0.0, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tank_depletes_by_exactly_the_delivered_volume() {
        let mut tank = TankModel::new(TankParams::default(), 100.0);
        assert_eq!(tank.remaining_ml(), 2_000.0);
        assert_eq!(tank.draw(80.0), 80.0);
        assert_eq!(tank.remaining_ml(), 1_920.0);
        assert!((tank.true_percent() - 96.0).abs() < 1e-9);
    }

    #[test]
    fn the_tank_never_goes_negative_and_reports_what_it_could_supply() {
        let mut tank = TankModel::new(TankParams::default(), 1.0);
        assert_eq!(tank.remaining_ml(), 20.0);
        assert_eq!(
            tank.draw(80.0),
            20.0,
            "a pump cannot deliver water that is not there"
        );
        assert_eq!(tank.remaining_ml(), 0.0);
        assert_eq!(tank.draw(80.0), 0.0);
        assert_eq!(tank.true_percent(), 0.0);
    }

    #[test]
    fn a_nonsense_draw_takes_nothing() {
        let mut tank = TankModel::new(TankParams::default(), 50.0);
        for ml in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(tank.draw(ml), 0.0);
        }
        assert_eq!(tank.remaining_ml(), 1_000.0);
    }

    #[test]
    fn the_leak_sensor_is_clear_until_something_injects_a_leak() {
        let mut tank = TankModel::new(TankParams::default(), 50.0);
        assert_eq!(tank.leak(), LeakState::Clear);
        tank.set_leak(LeakState::Detected);
        assert_eq!(tank.leak(), LeakState::Detected);
        tank.set_leak(LeakState::Unknown);
        assert_eq!(
            tank.leak(),
            LeakState::Unknown,
            "an unreadable sensor is Unknown, which is never permission"
        );
    }

    #[test]
    fn the_level_can_be_set_directly_and_is_clamped() {
        let mut tank = TankModel::new(TankParams::default(), 50.0);
        tank.set_percent(0.0);
        assert_eq!(tank.remaining_ml(), 0.0);
        tank.set_percent(250.0);
        assert_eq!(tank.true_percent(), 100.0);
        tank.set_percent(-10.0);
        assert_eq!(tank.true_percent(), 0.0);
    }

    #[test]
    fn a_reading_stays_inside_the_physical_range_even_with_noise() {
        let mut rng = SplitMix64::new(11);
        for level in [0.0, 50.0, 100.0] {
            let tank = TankModel::new(TankParams::default(), level);
            for _ in 0..1_000 {
                let reading = tank.sample_percent(&mut rng, true);
                assert!(
                    (0.0..=100.0).contains(&reading),
                    "{reading} is outside the kind's declared range"
                );
            }
        }
    }
}
