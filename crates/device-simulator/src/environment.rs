//! The pot, the reservoir, and the generator that perturbs their readings.
//!
//! Holds the four physical models together and routes a delivery through them
//! in the order the physics requires: the tank supplies what it can, the soil
//! keeps what fits and drains the rest, and the scale gains exactly what the
//! soil kept.

use crate::cli::Cli;
use crate::model::{
    EcModel, EcParams, SoilModel, SoilParams, TankModel, TankParams, WeightModel, WeightParams,
};
use crate::rng::SplitMix64;

/// Water mass held by a pot at a given moisture, used to seed the scale so the
/// two models start out consistent with each other.
fn initial_water_g(soil: &SoilParams, vwc: f64) -> f64 {
    soil.pot_volume_ml * soil.soil_factor * vwc / 100.0
}

/// What a delivery actually did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delivered {
    /// Volume the pump moved out of the reservoir. This is what the command
    /// result reports and what counts against the daily budget.
    pub delivered_ml: f64,
    /// Volume the soil kept.
    pub retained_ml: f64,
    /// Volume that ran straight through and will never be measured.
    pub drained_ml: f64,
}

/// The simulated world around one plant.
#[derive(Clone, Debug)]
pub struct Environment {
    /// Soil moisture and temperature.
    pub soil: SoilModel,
    /// The pot on its scale.
    pub weight: WeightModel,
    /// The reservoir and its leak sensor.
    pub tank: TankModel,
    /// The conductivity probe.
    pub ec: EcModel,
    /// The pack, where the device has one. Telemetry only (ADR-018 section 7).
    pub battery: crate::model::battery::BatteryModel,
    /// The deterministic generator every reading draws its noise from.
    pub rng: SplitMix64,
    /// Whether readings carry Gaussian noise.
    pub noise: bool,
}

impl Environment {
    /// Builds the world from validated command-line settings.
    #[must_use]
    pub fn from_cli(cli: &Cli) -> Self {
        let soil_params = SoilParams {
            drying_rate_per_hour: cli.drying_rate,
            pot_volume_ml: cli.pot_volume_ml,
            ..SoilParams::default()
        };
        let tank_params = TankParams {
            capacity_ml: cli.tank_capacity_ml,
            ..TankParams::default()
        };
        Self {
            weight: WeightModel::new(
                WeightParams::default(),
                initial_water_g(&soil_params, cli.initial_moisture),
            ),
            soil: SoilModel::new(soil_params, cli.initial_moisture),
            tank: TankModel::new(tank_params, 100.0),
            ec: EcModel::new(EcParams::default()),
            battery: crate::model::battery::BatteryModel::new(
                crate::model::battery::BatteryParams::default(),
                100.0,
            ),
            rng: SplitMix64::new(cli.seed),
            noise: !cli.no_noise,
        }
    }

    /// Advances every model by elapsed virtual time.
    pub fn step(&mut self, dt_ms: u64) {
        self.soil.step(dt_ms);
        self.weight.step(dt_ms);
        self.ec.step(dt_ms);
    }

    /// Runs water from the reservoir into the pot.
    ///
    /// `deliver_water` is the *physical* path and knows nothing about
    /// permission: whether a dose may happen at all is decided by
    /// `rhizo_mqtt_contract::validate_water_command` before this is reached
    /// (M2-008). Keeping the two apart is what makes the single-call-site
    /// property checkable — this function is not a second gate, and must never
    /// become one.
    pub fn deliver_water(&mut self, requested_ml: f64) -> Delivered {
        let delivered_ml = self.tank.draw(requested_ml);
        let split = self.soil.deliver(delivered_ml);
        self.weight.deliver(split.retained_ml);
        Delivered {
            delivered_ml,
            retained_ml: split.retained_ml,
            drained_ml: split.drained_ml,
        }
    }

    /// Runs the pump without moving water, for the `pump-no-delivery` fault.
    ///
    /// The reservoir does not fall, the soil does not wet, and the scale does
    /// not move — which is exactly the signature the weight-based no-delivery
    /// detection of M6-017 has to find.
    pub const fn deliver_nothing(&self) -> Delivered {
        Delivered {
            delivered_ml: 0.0,
            retained_ml: 0.0,
            drained_ml: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;

    #[test]
    fn the_scale_and_the_soil_start_out_consistent() {
        let environment = Environment::from_cli(&cli(&[
            "--initial-moisture",
            "40",
            "--pot-volume-ml",
            "2500",
        ]));
        assert_eq!(environment.soil.true_vwc(), 40.0);
        assert_eq!(
            environment.weight.water_g(),
            1_000.0,
            "40 % of 2500 ml is 1000 g of water"
        );
    }

    #[test]
    fn a_delivery_moves_water_from_the_tank_to_the_pot() {
        let mut environment = Environment::from_cli(&cli(&["--initial-moisture", "20"]));
        let tank_before = environment.tank.remaining_ml();
        let weight_before = environment.weight.water_g();

        let delivered = environment.deliver_water(50.0);

        assert_eq!(delivered.delivered_ml, 50.0);
        assert_eq!(delivered.drained_ml, 0.0);
        assert_eq!(environment.tank.remaining_ml(), tank_before - 50.0);
        assert_eq!(environment.weight.water_g(), weight_before + 50.0);
        assert_eq!(
            environment.soil.true_vwc(),
            20.0,
            "and the moisture reading still lags"
        );
    }

    #[test]
    fn an_empty_tank_delivers_only_what_it_has() {
        let mut environment = Environment::from_cli(&cli(&["--initial-moisture", "20"]));
        environment.tank.set_percent(0.5); // 10 ml at the default capacity
        let delivered = environment.deliver_water(80.0);
        assert_eq!(delivered.delivered_ml, 10.0);
        assert_eq!(environment.tank.remaining_ml(), 0.0);
    }

    #[test]
    fn water_beyond_field_capacity_leaves_the_pot_entirely() {
        let mut environment = Environment::from_cli(&cli(&["--initial-moisture", "44"]));
        let weight_before = environment.weight.water_g();
        let delivered = environment.deliver_water(80.0);
        assert!(delivered.drained_ml > 0.0);
        assert!(
            (environment.weight.water_g() - weight_before - delivered.retained_ml).abs() < 1e-9,
            "the scale gains only what the soil kept; drained water is gone"
        );
    }

    #[test]
    fn a_pump_that_delivers_nothing_leaves_every_model_untouched() {
        let environment = Environment::from_cli(&cli(&[]));
        let before = (
            environment.tank.remaining_ml(),
            environment.weight.water_g(),
            environment.soil.true_vwc(),
        );
        let delivered = environment.deliver_nothing();
        assert_eq!(delivered.delivered_ml, 0.0);
        assert_eq!(
            (
                environment.tank.remaining_ml(),
                environment.weight.water_g(),
                environment.soil.true_vwc()
            ),
            before
        );
    }

    #[test]
    fn the_whole_environment_is_deterministic_for_a_seed() {
        let run = || {
            let mut environment = Environment::from_cli(&cli(&["--seed", "99"]));
            let mut readings = Vec::new();
            for i in 0..100 {
                if i == 20 {
                    environment.deliver_water(60.0);
                }
                environment.step(60_000);
                let noise = environment.noise;
                readings.push(
                    environment
                        .soil
                        .sample_vwc(&mut environment.rng, noise)
                        .to_bits(),
                );
                readings.push(
                    environment
                        .weight
                        .sample_g(&mut environment.rng, noise)
                        .to_bits(),
                );
            }
            readings
        };
        assert_eq!(run(), run());
    }
}
