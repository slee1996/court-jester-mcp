//! Structured lifecycle events emitted by generated harnesses.

use crate::types::{HarnessEvent, HarnessEventRecord, InputClassification, UnitOutcome};
use std::collections::{BTreeMap, HashSet};

pub const HARNESS_EVENT_SENTINEL: &str = "__COURT_JESTER_EVENT_JSON__";
pub const HARNESS_EVENT_PROTOCOL_VERSION: u32 = 2;
pub const HARNESS_EVENT_MAX_LINE_BYTES: usize = 262_144;
pub const HARNESS_EVENT_MAX_RECORDS: usize = 100_000;

#[derive(Debug, Clone)]
pub struct HarnessEventSummary {
    pub records: Vec<HarnessEventRecord>,
    pub findings: Vec<crate::types::VerificationFinding>,
    pub completed_units: usize,
    pub runner_started: bool,
    pub target_resolved: bool,
    pub target_ready: bool,
    pub harness_completed: bool,
    pub open_unit: Option<(String, usize)>,
    pub surfaces: BTreeMap<String, HarnessSurfaceEvidence>,
}

/// Counts are derived from matched lifecycle records, never textual FUZZ lines.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct HarnessSurfaceEvidence {
    pub started: usize,
    pub completed: usize,
    pub valid_completed: usize,
    pub rejected: usize,
    pub invalid_completed: usize,
    pub unknown_completed: usize,
    pub passed_oracles: usize,
    pub failed_oracles: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    Start,
    Bootstrap,
    Resolved,
    Ready,
    Completed,
    BootstrapFailed,
}

fn event_protocol_error(message: impl Into<String>) -> String {
    format!("harness protocol error: {}", message.into())
}

pub fn parse_harness_events(output: &str) -> Result<HarnessEventSummary, String> {
    let mut records = Vec::new();
    let mut seen_sequences = HashSet::new();
    let mut state = EventState::Start;
    let mut next_sequence = 0u64;
    let mut stream_version = None;
    let mut current_unit: Option<(String, usize)> = None;
    let mut current_validity = InputClassification::Unknown;
    let mut current_checks = (0usize, 0usize);
    let mut seen_units = HashSet::new();
    let mut surfaces = BTreeMap::<String, HarnessSurfaceEvidence>::new();
    let mut findings = Vec::new();
    let mut completed_units = 0usize;

    for line in output.lines() {
        if !line
            .as_bytes()
            .starts_with(HARNESS_EVENT_SENTINEL.as_bytes())
        {
            continue;
        }
        if line.len() > HARNESS_EVENT_MAX_LINE_BYTES {
            return Err(event_protocol_error("event line exceeds 262144 bytes"));
        }
        if records.len() >= HARNESS_EVENT_MAX_RECORDS {
            return Err(event_protocol_error("event record limit exceeded"));
        }
        let payload = &line[HARNESS_EVENT_SENTINEL.len()..];
        if payload.starts_with(HARNESS_EVENT_SENTINEL) {
            return Err(event_protocol_error("duplicate event sentinel"));
        }
        let record = serde_json::from_str::<HarnessEventRecord>(payload)
            .map_err(|error| event_protocol_error(error.to_string()))?;
        if !matches!(record.protocol_version, 1 | HARNESS_EVENT_PROTOCOL_VERSION) {
            return Err(event_protocol_error(format!(
                "unsupported protocol version {}",
                record.protocol_version
            )));
        }
        if stream_version.is_some_and(|version| version != record.protocol_version) {
            return Err(event_protocol_error(
                "mixed protocol versions in one stream",
            ));
        }
        stream_version = Some(record.protocol_version);
        if !seen_sequences.insert(record.sequence) {
            return Err(event_protocol_error("duplicate event sequence"));
        }
        if record.sequence != next_sequence {
            return Err(event_protocol_error(format!(
                "expected sequence {}, got {}",
                next_sequence, record.sequence
            )));
        }
        next_sequence = next_sequence.saturating_add(1);
        match &record.event {
            HarnessEvent::BootstrapStarted => {
                if state != EventState::Start {
                    return Err(event_protocol_error("bootstrap_started is not first"));
                }
                state = EventState::Bootstrap;
            }
            HarnessEvent::TargetResolved { module } => {
                if state != EventState::Bootstrap || module.is_empty() {
                    return Err(event_protocol_error("target_resolved before bootstrap"));
                }
                state = EventState::Resolved;
            }
            HarnessEvent::BootstrapFailed { .. } => {
                if !matches!(state, EventState::Bootstrap | EventState::Resolved) {
                    return Err(event_protocol_error("bootstrap_failed in invalid state"));
                }
                state = EventState::BootstrapFailed;
            }
            HarnessEvent::TargetReady => {
                if state != EventState::Resolved {
                    return Err(event_protocol_error("target_ready before target_resolved"));
                }
                state = EventState::Ready;
            }
            HarnessEvent::UnitStarted {
                surface_id,
                iteration,
                input_classification,
                ..
            } => {
                if state != EventState::Ready || current_unit.is_some() {
                    return Err(event_protocol_error(
                        "unit_started overlaps or precedes target",
                    ));
                }
                if surface_id.is_empty() || !seen_units.insert((surface_id.clone(), *iteration)) {
                    return Err(event_protocol_error(
                        "empty surface or duplicate invocation identity",
                    ));
                }
                current_validity = *input_classification;
                current_checks = (0, 0);
                surfaces.entry(surface_id.clone()).or_default().started += 1;
                current_unit = Some((surface_id.clone(), *iteration));
            }
            HarnessEvent::OracleEvaluated {
                surface_id,
                iteration,
                oracle_id,
                passed,
            } => {
                if record.protocol_version < 2 {
                    return Err(event_protocol_error(
                        "oracle_evaluated requires protocol version 2",
                    ));
                }
                if current_unit.as_ref() != Some(&(surface_id.clone(), *iteration))
                    || oracle_id.is_empty()
                {
                    return Err(event_protocol_error(
                        "oracle_evaluated does not match an active unit or has an empty oracle id",
                    ));
                }
                if *passed {
                    current_checks.0 += 1;
                } else {
                    current_checks.1 += 1;
                }
            }
            HarnessEvent::Finding { finding } => {
                if current_unit.is_none() && state != EventState::Ready {
                    return Err(event_protocol_error(
                        "finding must be inside a unit or follow target_ready",
                    ));
                }
                findings.push(finding.clone());
            }
            HarnessEvent::UnitCompleted {
                surface_id,
                iteration,
                outcome,
            } => {
                if current_unit.as_ref() != Some(&(surface_id.clone(), *iteration)) {
                    return Err(event_protocol_error(
                        "unit_completed does not match unit_started",
                    ));
                }
                if *outcome == UnitOutcome::Passed && current_checks.1 > 0 {
                    return Err(event_protocol_error("passed unit contains a failed oracle"));
                }
                current_unit = None;
                let evidence = surfaces
                    .get_mut(surface_id)
                    .expect("matched start has a surface");
                evidence.completed += 1;
                match (current_validity, outcome) {
                    (_, UnitOutcome::UnclassifiedException) => evidence.unknown_completed += 1,
                    (_, UnitOutcome::Rejected) => evidence.rejected += 1,
                    (InputClassification::Invalid, _) | (_, UnitOutcome::InvalidGeneratedInput) => {
                        evidence.invalid_completed += 1
                    }
                    (InputClassification::Unknown, _) => evidence.unknown_completed += 1,
                    (
                        InputClassification::Valid,
                        UnitOutcome::Passed | UnitOutcome::TargetException,
                    ) => {
                        evidence.valid_completed += 1;
                        evidence.passed_oracles += current_checks.0;
                        evidence.failed_oracles += current_checks.1;
                    }
                }
                completed_units = completed_units.saturating_add(1);
            }
            HarnessEvent::HarnessCompleted {
                completed_units: reported,
            } => {
                if state != EventState::Ready || current_unit.is_some() {
                    return Err(event_protocol_error("harness_completed before units close"));
                }
                if *reported != completed_units {
                    return Err(event_protocol_error("harness_completed count disagrees"));
                }
                state = EventState::Completed;
            }
        }
        records.push(record);
    }

    if state == EventState::Start {
        return Err(event_protocol_error("no bootstrap event"));
    }

    Ok(HarnessEventSummary {
        records,
        findings,
        completed_units,
        runner_started: state != EventState::Start,
        target_resolved: matches!(
            state,
            EventState::Resolved | EventState::Ready | EventState::Completed
        ),
        target_ready: matches!(state, EventState::Ready | EventState::Completed),
        harness_completed: state == EventState::Completed,
        open_unit: current_unit,
        surfaces,
    })
}
