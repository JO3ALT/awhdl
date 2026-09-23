//! Completion is decided by the runtime from trusted facts. A planner's
//! `complete` decision only asks for this evaluation; it cannot satisfy a hard
//! condition, and no planner field can assert one.
use crate::effect::EffectState;
use crate::execution::ExecutionState;
use crate::state_machine::{ActionState, ActionStatus};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoftFailure {
    /// Hard conditions hold; hand the result to a human for review.
    #[default]
    RequiresReview,
    /// Keep working: the planner is told which soft conditions are unmet.
    Retry,
    /// Accept the result and record the unmet soft conditions.
    Complete,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionPolicy {
    /// Safety-critical workflows need explicit deterministic hard conditions.
    #[serde(default)]
    pub safety_critical: bool,
    #[serde(default)]
    pub hard: Vec<String>,
    #[serde(default)]
    pub soft: Vec<String>,
    #[serde(default)]
    pub on_soft_failure: SoftFailure,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// The trusted state machine recorded a successful run of the action.
    Succeeded(String),
    /// No Effect is STARTED or UNCERTAIN. Always implied as a hard condition.
    EffectsResolved,
    /// A typed adapter's structured output (not model prose) has this value.
    Structured {
        action: String,
        pointer: String,
        expected: Value,
    },
    /// The planner asked to complete. Model-derived: soft only.
    PlannerClaim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Deterministic,
    Model,
}

impl Condition {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text == "effects_resolved" {
            return Ok(Self::EffectsResolved);
        }
        if text == "planner_claim" {
            return Ok(Self::PlannerClaim);
        }
        if let Some(action) = text.strip_prefix("succeeded:") {
            ensure!(!action.is_empty(), "empty action in {text}");
            return Ok(Self::Succeeded(action.to_owned()));
        }
        if let Some(rest) = text.strip_prefix("structured:") {
            let (action, rest) = rest.split_once(':').with_context(|| {
                format!("expected structured:<action>:<pointer>=<json>: {text}")
            })?;
            let (pointer, expected) = rest
                .split_once('=')
                .with_context(|| format!("structured condition needs =<json>: {text}"))?;
            ensure!(
                !action.is_empty() && pointer.starts_with('/'),
                "invalid structured condition: {text}"
            );
            let expected = serde_json::from_str(expected)
                .with_context(|| format!("expected value is not JSON: {text}"))?;
            return Ok(Self::Structured {
                action: action.to_owned(),
                pointer: pointer.to_owned(),
                expected,
            });
        }
        bail!("unknown completion condition: {text}")
    }

    /// `is_model_route` tells whether an action's success is only model output.
    pub fn provenance(&self, is_model_route: impl Fn(&str) -> bool) -> Provenance {
        match self {
            Self::PlannerClaim => Provenance::Model,
            Self::Succeeded(action) if is_model_route(action) => Provenance::Model,
            _ => Provenance::Deterministic,
        }
    }

    pub fn action(&self) -> Option<&str> {
        match self {
            Self::Succeeded(action) | Self::Structured { action, .. } => Some(action),
            _ => None,
        }
    }
}

impl CompletionPolicy {
    pub fn hard_conditions(&self) -> Result<Vec<Condition>> {
        let mut conditions = self
            .hard
            .iter()
            .map(|text| Condition::parse(text))
            .collect::<Result<Vec<_>>>()?;
        if !conditions.contains(&Condition::EffectsResolved) {
            conditions.push(Condition::EffectsResolved);
        }
        Ok(conditions)
    }

    pub fn soft_conditions(&self) -> Result<Vec<Condition>> {
        self.soft
            .iter()
            .map(|text| Condition::parse(text))
            .collect()
    }

    /// Static rules: hard conditions are deterministic; safety-critical
    /// workflows declare at least one hard condition of their own.
    pub fn validate(&self, is_model_route: impl Fn(&str) -> bool) -> Result<()> {
        for condition in self.hard_conditions()? {
            ensure!(
                condition.provenance(&is_model_route) == Provenance::Deterministic,
                "model-derived condition {condition:?} cannot be a hard condition"
            );
        }
        self.soft_conditions()?;
        ensure!(
            !self.safety_critical || !self.hard.is_empty(),
            "a safety-critical completion policy needs explicit hard conditions"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatus {
    Complete,
    Incomplete,
    Blocked,
    Failed,
    RequiresReview,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionReport {
    pub status: CompletionStatus,
    pub unmet_hard: Vec<String>,
    pub unmet_soft: Vec<String>,
    pub reason: Option<String>,
}

/// Trusted inputs only: state machine, Effect journal, typed adapter output.
pub struct CompletionFacts<'a> {
    pub actions: &'a [ActionState],
    pub execution: &'a ExecutionState,
    pub structured: &'a BTreeMap<String, Value>,
    pub planner_claim: bool,
}

impl CompletionFacts<'_> {
    fn action(&self, name: &str) -> Option<&ActionState> {
        self.actions.iter().find(|state| state.action == name)
    }

    fn holds(&self, condition: &Condition) -> bool {
        match condition {
            Condition::Succeeded(action) => self
                .action(action)
                .is_some_and(|state| state.status == ActionStatus::Succeeded),
            Condition::EffectsResolved => self.unresolved_effects() == 0,
            Condition::Structured {
                action,
                pointer,
                expected,
            } => self
                .structured
                .get(action)
                .and_then(|value| value.pointer(pointer))
                .is_some_and(|actual| actual == expected),
            Condition::PlannerClaim => self.planner_claim,
        }
    }

    fn unresolved_effects(&self) -> usize {
        self.execution
            .effects()
            .filter(|effect| matches!(effect.state, EffectState::Started | EffectState::Uncertain))
            .count()
    }

    /// A hard condition that can no longer become true.
    fn unreachable(&self, condition: &Condition) -> bool {
        condition
            .action()
            .and_then(|name| self.action(name))
            .is_some_and(|state| {
                state.status == ActionStatus::Failed && state.attempts >= state.max_attempts
            })
    }
}

pub fn evaluate(
    policy: &CompletionPolicy,
    facts: &CompletionFacts<'_>,
) -> Result<CompletionReport> {
    let hard = policy.hard_conditions()?;
    let soft = policy.soft_conditions()?;
    let describe = |conditions: Vec<&Condition>| {
        conditions
            .into_iter()
            .map(|condition| format!("{condition:?}"))
            .collect::<Vec<_>>()
    };
    let unmet_hard = hard.iter().filter(|c| !facts.holds(c)).collect::<Vec<_>>();
    let unmet_soft = soft.iter().filter(|c| !facts.holds(c)).collect::<Vec<_>>();
    let (status, reason) = if facts.unresolved_effects() > 0 {
        (
            CompletionStatus::Blocked,
            Some("an Effect is unresolved; reconcile before completion".to_owned()),
        )
    } else if unmet_hard.iter().any(|c| facts.unreachable(c)) {
        (
            CompletionStatus::Failed,
            Some("a required action exhausted its attempts".to_owned()),
        )
    } else if !unmet_hard.is_empty() {
        (CompletionStatus::Incomplete, None)
    } else if unmet_soft.is_empty() {
        (CompletionStatus::Complete, None)
    } else {
        match policy.on_soft_failure {
            SoftFailure::RequiresReview => (CompletionStatus::RequiresReview, None),
            SoftFailure::Retry => (CompletionStatus::Incomplete, None),
            SoftFailure::Complete => (CompletionStatus::Complete, None),
        }
    };
    Ok(CompletionReport {
        status,
        unmet_hard: describe(unmet_hard),
        unmet_soft: describe(unmet_soft),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectClass, EffectDecision, EffectRequest};
    use serde_json::json;
    use uuid::Uuid;

    fn action(name: &str, status: ActionStatus, attempts: u32) -> ActionState {
        ActionState {
            action: name.to_owned(),
            status,
            attempts,
            max_attempts: 2,
            repeatable: false,
            depends_on: Vec::new(),
            last_error: None,
        }
    }

    fn policy(hard: &[&str], soft: &[&str], on_soft_failure: SoftFailure) -> CompletionPolicy {
        CompletionPolicy {
            safety_critical: true,
            hard: hard.iter().map(|s| s.to_string()).collect(),
            soft: soft.iter().map(|s| s.to_string()).collect(),
            on_soft_failure,
        }
    }

    fn status(
        policy: &CompletionPolicy,
        actions: &[ActionState],
        execution: &ExecutionState,
        structured: &BTreeMap<String, Value>,
    ) -> CompletionStatus {
        evaluate(
            policy,
            &CompletionFacts {
                actions,
                execution,
                structured,
                planner_claim: true,
            },
        )
        .unwrap()
        .status
    }

    #[test]
    fn hard_failure_blocks_completion_even_when_soft_and_planner_pass() {
        let execution = ExecutionState::new(Uuid::new_v4());
        let structured = BTreeMap::from([("proof".to_owned(), json!({"ok": false}))]);
        let policy = policy(
            &["succeeded:build", "structured:proof:/ok=true"],
            &["planner_claim"],
            SoftFailure::RequiresReview,
        );
        let actions = [
            action("build", ActionStatus::Succeeded, 1),
            action("proof", ActionStatus::Succeeded, 1),
        ];
        // The planner claimed completion, the soft condition holds, the proof did not.
        assert_eq!(
            status(&policy, &actions, &execution, &structured),
            CompletionStatus::Incomplete
        );
        let proved = BTreeMap::from([("proof".to_owned(), json!({"ok": true}))]);
        assert_eq!(
            status(&policy, &actions, &execution, &proved),
            CompletionStatus::Complete
        );
        let exhausted = [
            action("build", ActionStatus::Failed, 2),
            action("proof", ActionStatus::Succeeded, 1),
        ];
        assert_eq!(
            status(&policy, &exhausted, &execution, &proved),
            CompletionStatus::Failed
        );
        let retryable = [
            action("build", ActionStatus::Failed, 1),
            action("proof", ActionStatus::Succeeded, 1),
        ];
        assert_eq!(
            status(&policy, &retryable, &execution, &proved),
            CompletionStatus::Incomplete
        );
    }

    #[test]
    fn soft_failure_follows_policy() {
        let execution = ExecutionState::new(Uuid::new_v4());
        let structured = BTreeMap::new();
        let actions = [
            action("tests", ActionStatus::Succeeded, 1),
            action("review", ActionStatus::Failed, 1),
        ];
        let check = |on_soft_failure| {
            status(
                &policy(&["succeeded:tests"], &["succeeded:review"], on_soft_failure),
                &actions,
                &execution,
                &structured,
            )
        };
        assert_eq!(
            check(SoftFailure::RequiresReview),
            CompletionStatus::RequiresReview
        );
        assert_eq!(check(SoftFailure::Retry), CompletionStatus::Incomplete);
        assert_eq!(check(SoftFailure::Complete), CompletionStatus::Complete);
    }

    #[test]
    fn unresolved_effect_blocks_completion_implicitly() {
        let mut execution = ExecutionState::new(Uuid::new_v4());
        let request = EffectRequest::new(
            "action:send",
            "send",
            "mail:send",
            &json!({}),
            EffectClass::ExternalWrite,
            None,
            true,
        )
        .unwrap()
        .with_approval(false)
        .unwrap();
        let EffectDecision::Dispatch { effect_id, .. } =
            execution.reserve_effect(&request).unwrap()
        else {
            panic!("dispatch")
        };
        let invocation = execution.begin("action:send", None).unwrap();
        execution.start_effect(effect_id, &invocation).unwrap();
        execution
            .finish_effect(effect_id, &invocation, None)
            .unwrap();
        // No hard condition was declared, yet an UNCERTAIN write blocks completion.
        let lenient = CompletionPolicy::default();
        assert_eq!(
            status(&lenient, &[], &execution, &BTreeMap::new()),
            CompletionStatus::Blocked
        );
    }

    #[test]
    fn model_derived_conditions_cannot_be_hard() {
        let is_model = |action: &str| action == "reviewer";
        assert!(
            policy(&["planner_claim"], &[], SoftFailure::RequiresReview)
                .validate(is_model)
                .is_err()
        );
        assert!(
            policy(&["succeeded:reviewer"], &[], SoftFailure::RequiresReview)
                .validate(is_model)
                .is_err()
        );
        assert!(
            policy(
                &["succeeded:tests"],
                &["succeeded:reviewer", "planner_claim"],
                SoftFailure::Retry
            )
            .validate(is_model)
            .is_ok()
        );
        let unsafe_empty = CompletionPolicy {
            safety_critical: true,
            ..CompletionPolicy::default()
        };
        assert!(unsafe_empty.validate(is_model).is_err());
        assert!(CompletionPolicy::default().validate(is_model).is_ok());
        for bad in [
            "done",
            "succeeded:",
            "structured:a:ok=true",
            "structured:a:/ok=yes",
        ] {
            assert!(Condition::parse(bad).is_err(), "{bad}");
        }
    }
}
