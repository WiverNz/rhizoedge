//! The fault set.
//!
//! Holds which faults are enabled and answers the questions the rest of the
//! simulator asks of them. Faults **compose**: a leak and an empty tank at once
//! is a real situation, and a set rather than a single "current fault" is what
//! makes it expressible.
//!
//! # Faults never bypass anything
//!
//! Every fault here makes the device behave *worse*, not more permissive. A
//! fault can flood a tray, empty a reservoir, freeze a sensor, or kill the
//! process — it cannot deliver a dose, and it cannot make the shared gate say
//! yes. Injection goes in at the same places real failure would.

use std::collections::BTreeMap;

use crate::cli::{Fault, PolicyStep};
use crate::rng::SplitMix64;

/// The faults currently enabled, keyed by name so re-enabling one with a
/// different parameter replaces it rather than stacking.
#[derive(Clone, Debug, Default)]
pub struct FaultSet {
    enabled: BTreeMap<&'static str, Fault>,
}

impl FaultSet {
    /// Builds a set from the startup flags.
    #[must_use]
    pub fn from_flags(faults: &[Fault]) -> Self {
        let mut set = Self::default();
        for fault in faults {
            set.enable(*fault);
        }
        set
    }

    /// Enables a fault, replacing any earlier one of the same name.
    pub fn enable(&mut self, fault: Fault) {
        self.enabled.insert(fault.name(), fault);
    }

    /// Disables a fault by name. Returns whether anything was enabled.
    pub fn disable(&mut self, name: &str) -> bool {
        self.enabled.remove(name).is_some()
    }

    /// Whether a fault of this name is enabled.
    #[must_use]
    pub fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains_key(name)
    }

    /// The enabled fault of a given name, with its parameter.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Fault> {
        self.enabled.get(name).copied()
    }

    /// Every enabled fault, in a stable order.
    pub fn active(&self) -> impl Iterator<Item = Fault> + '_ {
        self.enabled.values().copied()
    }

    /// How many faults are enabled.
    #[must_use]
    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    /// Whether nothing is enabled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// The rate of a rate-based fault, or zero when it is not enabled.
    #[must_use]
    pub fn rate(&self, name: &str) -> f64 {
        match self.get(name) {
            Some(
                Fault::Duplicate { rate } | Fault::Reorder { rate } | Fault::InvalidSoil { rate },
            ) => rate,
            _ => 0.0,
        }
    }

    /// Whether a rate-based fault fires this time.
    ///
    /// Draws from the deterministic generator, so a seeded run reproduces
    /// exactly which messages were duplicated and which readings were spoiled.
    pub fn fires(&self, name: &str, rng: &mut SplitMix64) -> bool {
        rng.chance(self.rate(name))
    }

    /// The clock offset in milliseconds, zero when `clock-skew` is not enabled.
    #[must_use]
    pub fn clock_skew_ms(&self) -> i64 {
        match self.get("clock-skew") {
            Some(Fault::ClockSkew { seconds }) => seconds.saturating_mul(1_000),
            _ => 0,
        }
    }

    /// The isolation duration requested by `disconnect`, in milliseconds.
    #[must_use]
    pub fn disconnect_ms(&self) -> Option<u64> {
        match self.get("disconnect") {
            Some(Fault::Disconnect { seconds }) => Some(u64::from(seconds) * 1_000),
            _ => None,
        }
    }

    /// How many wake cycles `miss-wake` should skip.
    #[must_use]
    pub fn miss_wakes(&self) -> Option<u32> {
        match self.get("miss-wake") {
            Some(Fault::MissWake { count }) => Some(count),
            _ => None,
        }
    }

    /// The policy step the process should die after, if any.
    #[must_use]
    pub fn policy_interrupt(&self) -> Option<PolicyStep> {
        match self.get("policy-interrupt") {
            Some(Fault::PolicyInterrupt { step }) => Some(step),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faults_compose() {
        let set = FaultSet::from_flags(&[Fault::Leak, Fault::TankEmpty, Fault::PumpNoDelivery]);
        assert!(set.is_enabled("leak"));
        assert!(set.is_enabled("tank-empty"));
        assert!(set.is_enabled("pump-no-delivery"));
        assert_eq!(
            set.len(),
            3,
            "a leak and an empty tank at once is a real situation"
        );
    }

    #[test]
    fn re_enabling_replaces_rather_than_stacking() {
        let mut set = FaultSet::default();
        set.enable(Fault::Duplicate { rate: 0.1 });
        set.enable(Fault::Duplicate { rate: 0.9 });
        assert_eq!(set.len(), 1);
        assert_eq!(set.rate("duplicate"), 0.9);
    }

    #[test]
    fn disabling_reports_whether_anything_was_enabled() {
        let mut set = FaultSet::from_flags(&[Fault::Leak]);
        assert!(set.disable("leak"));
        assert!(
            !set.disable("leak"),
            "disabling twice is not an error, just a no-op"
        );
        assert!(set.is_empty());
    }

    #[test]
    fn an_unenabled_rate_is_zero_and_never_fires() {
        let set = FaultSet::default();
        let mut rng = SplitMix64::new(1);
        assert_eq!(set.rate("duplicate"), 0.0);
        for _ in 0..1_000 {
            assert!(!set.fires("duplicate", &mut rng));
        }
    }

    #[test]
    fn a_rate_of_one_always_fires_and_is_reproducible() {
        let set = FaultSet::from_flags(&[Fault::InvalidSoil { rate: 1.0 }]);
        let mut rng = SplitMix64::new(1);
        for _ in 0..100 {
            assert!(set.fires("invalid-soil", &mut rng));
        }

        let set = FaultSet::from_flags(&[Fault::Duplicate { rate: 0.5 }]);
        let draw = |seed| {
            let mut rng = SplitMix64::new(seed);
            (0..64)
                .map(|_| set.fires("duplicate", &mut rng))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            draw(9),
            draw(9),
            "a seeded run reproduces exactly which fired"
        );
        assert_ne!(draw(9), draw(10));
    }

    #[test]
    fn parameterised_faults_report_their_parameters() {
        let set = FaultSet::from_flags(&[
            Fault::ClockSkew { seconds: -90 },
            Fault::Disconnect { seconds: 30 },
            Fault::PolicyInterrupt {
                step: PolicyStep::Verify,
            },
        ]);
        assert_eq!(set.clock_skew_ms(), -90_000);
        assert_eq!(set.disconnect_ms(), Some(30_000));
        assert_eq!(set.policy_interrupt(), Some(PolicyStep::Verify));
    }

    #[test]
    fn absent_parameterised_faults_report_neutral_values() {
        let set = FaultSet::default();
        assert_eq!(set.clock_skew_ms(), 0);
        assert_eq!(set.disconnect_ms(), None);
        assert_eq!(set.policy_interrupt(), None);
    }

    #[test]
    fn every_catalogued_fault_can_be_enabled_and_named_back() {
        let all = [
            Fault::Disconnect { seconds: 1 },
            Fault::Duplicate { rate: 0.5 },
            Fault::Reorder { rate: 0.5 },
            Fault::InvalidSoil { rate: 0.5 },
            Fault::StuckSensor,
            Fault::ClockUnsync,
            Fault::ClockSkew { seconds: 1 },
            Fault::Leak,
            Fault::TankEmpty,
            Fault::PumpNoDelivery,
            Fault::PumpStuckOn,
            Fault::RestartMidDose,
            Fault::Restart,
            Fault::PolicyInterrupt {
                step: PolicyStep::Stage,
            },
        ];
        let set = FaultSet::from_flags(&all);
        assert_eq!(set.len(), all.len(), "every fault has a distinct name");
        for fault in all {
            assert!(set.is_enabled(fault.name()), "{fault} was not enabled");
        }
    }
}

/// Applies the transport faults — `duplicate` and `reorder` — to outgoing
/// publications.
///
/// A separate, pure type so the two faults are unit-testable without a broker,
/// and so the driver has one place that can perturb what goes out.
///
/// `duplicate` republishes with an **identical `message_id`**. A fresh id would
/// be a different message and would not test deduplication at all — it would
/// test that the edge stores two things, which it should. The whole point is a
/// redelivery the edge must collapse.
#[derive(Debug, Default)]
pub struct PublicationPipeline {
    /// A publication held back to arrive after the one that follows it.
    delayed: Option<crate::envelope::Publication>,
}

impl PublicationPipeline {
    /// A pipeline with nothing held back.
    #[must_use]
    pub const fn new() -> Self {
        Self { delayed: None }
    }

    /// Whether a publication is currently held back.
    #[must_use]
    pub const fn is_holding(&self) -> bool {
        self.delayed.is_some()
    }

    /// Passes publications through the enabled transport faults.
    pub fn process(
        &mut self,
        faults: &FaultSet,
        rng: &mut SplitMix64,
        incoming: Vec<crate::envelope::Publication>,
    ) -> Vec<crate::envelope::Publication> {
        let mut outgoing = Vec::with_capacity(incoming.len());
        for publication in incoming {
            // Only one message is ever held: "delayed past the next" is a
            // one-place swap, and holding an unbounded queue would be a
            // different fault (and an unbounded buffer on a device).
            if self.delayed.is_none() && faults.fires("reorder", rng) {
                self.delayed = Some(publication);
                continue;
            }
            emit(&mut outgoing, publication, faults, rng);
            if let Some(held) = self.delayed.take() {
                emit(&mut outgoing, held, faults, rng);
            }
        }
        outgoing
    }

    /// Releases anything still held.
    ///
    /// Called on shutdown so a delayed message is not simply lost: `reorder`
    /// reorders, it does not drop.
    pub fn flush(&mut self) -> Vec<crate::envelope::Publication> {
        self.delayed.take().into_iter().collect()
    }
}

/// Emits one publication, duplicating it if the fault fires.
fn emit(
    outgoing: &mut Vec<crate::envelope::Publication>,
    publication: crate::envelope::Publication,
    faults: &FaultSet,
    rng: &mut SplitMix64,
) {
    let duplicate = faults.fires("duplicate", rng);
    if duplicate {
        outgoing.push(publication.clone());
    }
    outgoing.push(publication);
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use crate::cli::Fault;
    use crate::envelope::Publication;
    use rhizo_mqtt_contract::{DeviceId, Topic};

    fn publication(n: usize) -> Publication {
        Publication::new(
            Topic::Telemetry(DeviceId::parse("plant-node-01").unwrap()),
            format!(r#"{{"message_id":"id-{n}"}}"#),
        )
    }

    fn batch(count: usize) -> Vec<Publication> {
        (0..count).map(publication).collect()
    }

    fn ids(publications: &[Publication]) -> Vec<String> {
        publications.iter().map(|p| p.payload.clone()).collect()
    }

    #[test]
    fn with_no_faults_nothing_is_perturbed() {
        let mut pipeline = PublicationPipeline::new();
        let mut rng = SplitMix64::new(1);
        let out = pipeline.process(&FaultSet::default(), &mut rng, batch(5));
        assert_eq!(ids(&out), ids(&batch(5)));
        assert!(!pipeline.is_holding());
    }

    #[test]
    fn duplicate_republishes_with_an_identical_message_id() {
        let faults = FaultSet::from_flags(&[Fault::Duplicate { rate: 1.0 }]);
        let mut pipeline = PublicationPipeline::new();
        let mut rng = SplitMix64::new(1);
        let out = pipeline.process(&faults, &mut rng, batch(3));
        assert_eq!(out.len(), 6, "every message published twice");
        for pair in out.chunks(2) {
            assert_eq!(
                pair[0].payload, pair[1].payload,
                "a fresh id would be a different message and would test nothing"
            );
        }
    }

    #[test]
    fn reorder_delays_a_message_past_the_next_one() {
        let faults = FaultSet::from_flags(&[Fault::Reorder { rate: 1.0 }]);
        let mut pipeline = PublicationPipeline::new();
        let mut rng = SplitMix64::new(1);
        let out = pipeline.process(&faults, &mut rng, batch(4));
        // The first is held; the second is emitted, then the first; the third
        // is then held in its place, and so on.
        assert_eq!(
            ids(&out),
            vec![
                publication(1).payload,
                publication(0).payload,
                publication(3).payload,
                publication(2).payload,
            ]
        );
        assert!(!pipeline.is_holding());
    }

    #[test]
    fn reorder_never_drops_a_message() {
        let faults = FaultSet::from_flags(&[Fault::Reorder { rate: 1.0 }]);
        let mut pipeline = PublicationPipeline::new();
        let mut rng = SplitMix64::new(1);
        let mut out = pipeline.process(&faults, &mut rng, batch(3));
        assert!(pipeline.is_holding(), "an odd count leaves one held");
        out.extend(pipeline.flush());
        let mut delivered = ids(&out);
        delivered.sort();
        let mut expected = ids(&batch(3));
        expected.sort();
        assert_eq!(delivered, expected, "reorder reorders; it does not drop");
    }

    #[test]
    fn the_two_transport_faults_compose() {
        let faults =
            FaultSet::from_flags(&[Fault::Reorder { rate: 1.0 }, Fault::Duplicate { rate: 1.0 }]);
        let mut pipeline = PublicationPipeline::new();
        let mut rng = SplitMix64::new(1);
        let out = pipeline.process(&faults, &mut rng, batch(2));
        assert_eq!(out.len(), 4, "one delayed pair, each duplicated");
        assert_eq!(out[0].payload, out[1].payload);
        assert_eq!(out[2].payload, out[3].payload);
        assert_eq!(out[0].payload, publication(1).payload);
        assert_eq!(out[2].payload, publication(0).payload);
    }

    #[test]
    fn a_seeded_run_reproduces_the_same_perturbation() {
        let faults =
            FaultSet::from_flags(&[Fault::Reorder { rate: 0.4 }, Fault::Duplicate { rate: 0.4 }]);
        let run = |seed| {
            let mut pipeline = PublicationPipeline::new();
            let mut rng = SplitMix64::new(seed);
            let mut out = pipeline.process(&faults, &mut rng, batch(40));
            out.extend(pipeline.flush());
            ids(&out)
        };
        assert_eq!(run(2026), run(2026));
        assert_ne!(run(2026), run(2027));
    }
}
