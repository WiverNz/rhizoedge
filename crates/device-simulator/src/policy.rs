//! Offline-policy persistence and atomic activation.
//!
//! [ADR-015](../../../../docs/adr/015-device-offline-autonomy.md) §7 requires
//! **validate → stage → verify → activate → acknowledge**, and forbids a device
//! from using a policy before activation completes (SAFETY-019).
//!
//! # What this module does *not* do
//!
//! It does not evaluate a policy, and it does not schedule a dose. An enabled,
//! valid, activated policy is completely inert in M2. The single shared
//! `rhizo_policy::evaluate_offline` and the simulator's one call site arrive
//! together in M6-019; writing rules here would create exactly the
//! simulator-specific evaluator ADR-008 exists to prevent, and every offline
//! safety test in M6 would then be exercising rules the hardware does not
//! follow.
//!
//! # The dangerous failure is a half-written policy taking effect
//!
//! A device acting on a dose field from the new policy and a cooldown from the
//! old one is the failure this sequence prevents. Power loss at any step leaves
//! **exactly one** valid policy active: the previous one before the activation
//! write, the new one after. Staging is a separate region, so nothing before
//! that write is destructive.
//!
//! # Rejection is non-destructive
//!
//! An invalid policy leaves the previous one active and reports the rejection.
//! The device keeps announcing the version it is actually running, so the edge
//! sees drift rather than a silent acceptance.

use std::collections::BTreeMap;

use rhizo_mqtt_contract::payload::{
    MeasurementKind, MeasurementPoint, OfflinePolicy, OfflinePolicySet,
    PolicyError as ContractPolicyError, SensorId,
};

use crate::capabilities::Capabilities;
use crate::cli::PolicyStep;
use crate::device::Device;
use crate::envelope::Publication;
use crate::state::StoredPolicy;
use rhizo_mqtt_contract::payload::DeviceStatusValue;

/// Why a policy was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyRejection {
    /// The payload could not be decoded.
    Malformed(&'static str),
    /// A bound the shared contract enforces was violated.
    Contract {
        /// Which plant.
        plant_id: String,
        /// Which bound.
        error: ContractPolicyError,
    },
    /// The policy named an actuator this device never declared.
    UndeclaredActuator {
        /// Which plant.
        plant_id: String,
        /// The actuator it named.
        actuator_id: String,
    },
    /// The policy referenced a measurement this device cannot produce.
    UnproducibleMeasurement {
        /// Which plant.
        plant_id: String,
        /// The kind it named.
        kind: String,
        /// The point it named.
        point: String,
    },
    /// The staged blob did not read back intact.
    StagingVerificationFailed,
    /// The state file could not be written.
    NotPersisted(String),
}

impl PolicyRejection {
    /// A stable label for logs and metrics.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "malformed",
            Self::Contract { .. } => "hard_limit_or_bound",
            Self::UndeclaredActuator { .. } => "undeclared_actuator",
            Self::UnproducibleMeasurement { .. } => "unproducible_measurement",
            Self::StagingVerificationFailed => "staging_verification_failed",
            Self::NotPersisted(_) => "not_persisted",
        }
    }
}

/// What happened to an incoming `device.policy`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyOutcome {
    /// Validated, staged, verified, activated, and acknowledged.
    Activated {
        /// The applied version per plant, after the merge.
        versions: BTreeMap<String, u32>,
    },
    /// Nothing in the message was newer than what is already applied.
    NothingNewer,
    /// Refused; the previous policy is untouched.
    Rejected(PolicyRejection),
}

/// Validates one policy against what this device actually declares.
///
/// The shared contract already checks the hard limits and the numeric bounds;
/// this adds the two checks only the device can make, because only the device
/// knows what hardware it has.
///
/// A **disabled** policy carries no rules to check, and the contract returns
/// early for one. The capability checks follow the same rule: a disabled policy
/// is inert, and re-enabling it requires a higher version, which is validated
/// again at that point.
///
/// # Errors
///
/// Returns the first violation found.
pub fn validate_against_capabilities(
    policy: &OfflinePolicy,
    capabilities: &Capabilities,
) -> Result<(), PolicyRejection> {
    let plant_id = policy.plant_id.as_str().to_owned();
    policy
        .validate()
        .map_err(|error| PolicyRejection::Contract {
            plant_id: plant_id.clone(),
            error,
        })?;
    if !policy.enabled {
        return Ok(());
    }

    if let Some(actuator) = policy.actuator.as_ref()
        && !capabilities.declares_actuator(&actuator.actuator_id)
    {
        return Err(PolicyRejection::UndeclaredActuator {
            plant_id,
            actuator_id: actuator.actuator_id.as_str().to_owned(),
        });
    }

    // ADR-015 §7 step 2: "every referenced kind/point is producible by a
    // declared sensor". Advisory measurements are included even though they
    // never gate actuation — a policy naming a sensor the device does not have
    // is a provisioning mistake, and refusing it is how the operator finds out
    // now rather than during an isolation.
    let referenced = std::iter::once((
        &policy.control_measurement.kind,
        &policy.control_measurement.point,
    ))
    .chain(
        policy
            .required_measurements
            .iter()
            .map(|m| (&m.kind, &m.point)),
    )
    .chain(
        policy
            .advisory_measurements
            .iter()
            .map(|m| (&m.kind, &m.point)),
    );
    for (kind, point) in referenced {
        if !capabilities.produces(kind, point) {
            return Err(PolicyRejection::UnproducibleMeasurement {
                plant_id,
                kind: kind.as_str().to_owned(),
                point: point.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

/// Merges accepted policies into the active set, replacing per plant.
///
/// Per plant, never wholesale. Protocol §5.11: removing a plant's policy is an
/// `enabled: false` republish at a higher version, **not** an omission — "an
/// omitted plant retains its last policy, because a dropped MQTT message must
/// not be able to silently disable, or silently enable, autonomy".
#[must_use]
pub fn merge(active: &OfflinePolicySet, accepted: Vec<OfflinePolicy>) -> OfflinePolicySet {
    let mut policies = active.policies.clone();
    for policy in accepted {
        match policies
            .iter_mut()
            .find(|p| p.plant_id.as_str() == policy.plant_id.as_str())
        {
            Some(existing) => *existing = policy,
            None => policies.push(policy),
        }
    }
    policies.sort_by(|a, b| a.plant_id.as_str().cmp(b.plant_id.as_str()));
    OfflinePolicySet { policies }
}

/// Selects the policies in a message that are strictly newer than what is
/// applied.
///
/// A version at or below the applied one is **ignored without validation**.
/// Without this, a retained republication after a rollback silently regresses a
/// device to an older policy, and nothing in the system reports it.
#[must_use]
pub fn newer_than_applied(
    incoming: &OfflinePolicySet,
    applied: &BTreeMap<String, u32>,
) -> Vec<OfflinePolicy> {
    incoming
        .policies
        .iter()
        .filter(|policy| {
            applied
                .get(policy.plant_id.as_str())
                .is_none_or(|version| policy.policy_version > *version)
        })
        .cloned()
        .collect()
}

/// The applied versions implied by an active set.
#[must_use]
pub fn versions_of(set: &OfflinePolicySet) -> BTreeMap<String, u32> {
    set.policies
        .iter()
        .map(|p| (p.plant_id.as_str().to_owned(), p.policy_version))
        .collect()
}

/// Convenience for tests and callers that need a kind/point pair by name.
#[must_use]
pub fn reference(
    kind: MeasurementKind,
    point: &str,
) -> Option<(MeasurementKind, MeasurementPoint)> {
    MeasurementPoint::parse(point).ok().map(|p| (kind, p))
}

/// Convenience for building a validated actuator reference.
#[must_use]
pub fn actuator_id(value: &str) -> Option<SensorId> {
    SensorId::parse(value).ok()
}

// ---------------------------------------------------------------- M2-016

impl Device {
    /// Applies a retained `device.policy` through ADR-015 §7's sequence.
    ///
    /// `validate → stage → verify → activate → acknowledge`, with the process
    /// interruptible at each boundary by `--fault policy-interrupt:<step>`. The
    /// step boundaries are real writes, not markers: interrupting between them
    /// leaves the state file in exactly the condition a power cut would.
    pub(crate) fn on_policy(&mut self, payload: &[u8]) -> Vec<Publication> {
        let Some(envelope) = self.decode::<OfflinePolicySet>(payload, "device.policy") else {
            // Step 1 failed: keep the active policy, report, stop.
            return self.report_policy_rejection(PolicyRejection::Malformed("undecodable payload"));
        };
        let incoming = envelope.data;

        // A version at or below the applied one is ignored *without* being
        // validated: it is not an error, it is a retained republication.
        let applied = self.store().state().applied_policy_versions.clone();
        let candidates = newer_than_applied(&incoming, &applied);
        if candidates.is_empty() {
            tracing::info!(
                applied = ?applied,
                offered = incoming.policies.len(),
                "policy ignored: nothing newer than the applied versions"
            );
            return Vec::new();
        }

        // ---- step 2: validate ------------------------------------------
        //
        // The whole message, not policy by policy. A half-applied set is the
        // failure this sequence exists to prevent, so one bad plant rejects
        // the message and leaves every plant on what it was already running.
        for policy in &candidates {
            if let Err(rejection) = validate_against_capabilities(policy, self.capabilities()) {
                return self.report_policy_rejection(rejection);
            }
        }
        if let Some(publications) = self.interrupt_policy_at(PolicyStep::Validate) {
            return publications;
        }

        let previous = self.store().state().policy_active.clone().map_or_else(
            || OfflinePolicySet {
                policies: Vec::new(),
            },
            |stored| stored.payload,
        );
        let merged = merge(&previous, candidates);
        let versions = versions_of(&merged);
        let staged = StoredPolicy::new(merged, versions.clone());

        // ---- step 3: write to staging, with a checksum -----------------
        //
        // A separate region. Nothing about this write can damage the active
        // policy, which is what makes every step up to activation reversible.
        if let Err(e) = self
            .store_mut()
            .mutate(|state| state.policy_staging = Some(staged.clone()))
        {
            return self.report_policy_rejection(PolicyRejection::NotPersisted(e.to_string()));
        }
        if let Some(publications) = self.interrupt_policy_at(PolicyStep::Stage) {
            return publications;
        }

        // ---- step 4: read back and verify ------------------------------
        //
        // From disk, not from memory. Verifying the copy still held in RAM
        // would prove only that the struct is intact — it would say nothing
        // about what actually reached storage, which is the thing that has to
        // survive the power cut.
        let verified = self.store().read_back().is_ok_and(|state| {
            state
                .policy_staging
                .as_ref()
                .is_some_and(|stored| stored.verify() && *stored == staged)
        });
        if !verified {
            tracing::error!("staged policy did not read back intact; keeping the active policy");
            // Clear the staging region: a blob that failed verification must
            // not be left where a later activation could pick it up.
            if let Err(e) = self.store_mut().mutate(|state| state.policy_staging = None) {
                tracing::error!(error = %e, "could not clear the failed staging region");
            }
            return self.report_policy_rejection(PolicyRejection::StagingVerificationFailed);
        }
        if let Some(publications) = self.interrupt_policy_at(PolicyStep::Verify) {
            return publications;
        }

        // ---- step 5: activate atomically -------------------------------
        //
        // One write moves the staged blob to active and records the applied
        // versions. The state file is written atomically, so there is no
        // instant at which the device holds half of each.
        if let Err(e) = self.store_mut().mutate(|state| {
            state.policy_active = state.policy_staging.take();
            state.applied_policy_versions = versions.clone();
        }) {
            return self.report_policy_rejection(PolicyRejection::NotPersisted(e.to_string()));
        }
        tracing::info!(
            versions = ?versions,
            "offline policy activated (inert until M6-019 installs the shared evaluator)"
        );
        // A real M2 audit event: the operator's record that this device began
        // holding this policy, replayed like any other history.
        for (plant_id, version) in &versions {
            let _ = plant_id;
            self.record_event(
                rhizo_mqtt_contract::payload::EventTier::Audit,
                rhizo_mqtt_contract::payload::EventKind::PolicyActivated,
                rhizo_mqtt_contract::payload::EventDetail::PolicyActivated {
                    policy_version: *version,
                },
            );
        }
        if let Some(publications) = self.interrupt_policy_at(PolicyStep::Activate) {
            return publications;
        }

        // ---- step 6: acknowledge ---------------------------------------
        let publications = match self.status_publication(DeviceStatusValue::Online, None) {
            Ok(publication) => vec![publication],
            Err(e) => {
                tracing::error!(error = %e, "could not acknowledge the applied policy");
                Vec::new()
            }
        };
        if let Some(interrupted) = self.interrupt_policy_at(PolicyStep::Acknowledge) {
            return interrupted;
        }
        publications
    }

    /// Kills the device at a chosen step of the activation sequence.
    ///
    /// Returns `Some` when the fault fired, so the caller stops immediately.
    /// One-shot: the fault disables itself first, or the restarted device would
    /// die again at the same step on the retained policy it receives on
    /// reconnect, forever.
    fn interrupt_policy_at(&mut self, step: PolicyStep) -> Option<Vec<Publication>> {
        if self.faults().policy_interrupt() != Some(step) {
            return None;
        }
        tracing::warn!(%step, "policy-interrupt: terminating during policy activation");
        self.disable_fault("policy-interrupt");
        self.restart();
        Some(Vec::new())
    }

    /// Reports a refused policy, leaving the active one untouched.
    fn report_policy_rejection(&mut self, rejection: PolicyRejection) -> Vec<Publication> {
        tracing::warn!(
            reason = rejection.reason(),
            ?rejection,
            applied = ?self.store().state().applied_policy_versions,
            "offline policy rejected; the previous policy stays active"
        );
        self.last_policy_rejection = Some(rejection);
        // The device keeps announcing the version it is actually running, so
        // the edge sees drift rather than a silent acceptance.
        Vec::new()
    }

    /// The most recent policy rejection, for tests and the control API.
    #[must_use]
    pub const fn last_policy_rejection(&self) -> Option<&PolicyRejection> {
        self.last_policy_rejection.as_ref()
    }

    /// The offline policy currently in force, if any.
    ///
    /// **Read-only in M2.** It is the input M6-019's single call site to
    /// `rhizo_policy::evaluate_offline` will take; nothing in M2 acts on it.
    #[must_use]
    pub fn active_policy(&self) -> Option<&OfflinePolicySet> {
        self.store()
            .state()
            .policy_active
            .as_ref()
            .map(|stored| &stored.payload)
    }

    /// The applied policy version per plant, as reported in status.
    #[must_use]
    pub fn applied_policy_versions(&self) -> &std::collections::BTreeMap<String, u32> {
        &self.store().state().applied_policy_versions
    }
}
