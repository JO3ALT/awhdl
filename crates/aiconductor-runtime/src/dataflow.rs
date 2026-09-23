//! Value storage and one-shot runtime events. This is not an AWHDL scheduler.
use crate::execution::{ExecutionState, InvocationStatus};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub workflow_run_id: Uuid,
    pub correlation_id: Uuid,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value<T> {
    pub scope: Scope,
    pub revision: u64,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    External {
        topic: String,
    },
    ValueChanged {
        name: String,
        revision: u64,
    },
    InvocationTerminated {
        invocation_id: Uuid,
        status: InvocationStatus,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event<T> {
    pub event_id: Uuid,
    pub sequence: u64,
    pub scope: Scope,
    pub causation_id: Option<Uuid>,
    pub kind: EventKind,
    pub payload: T,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DataflowState {
    values: BTreeMap<String, Value<Json>>,
    events: Vec<Event<Json>>,
    consumed: BTreeSet<Uuid>,
}

impl ExecutionState {
    /// Latest value in the current generation only; reads never consume events.
    pub fn value(&self, name: &str) -> Option<&Value<Json>> {
        self.dataflow
            .values
            .get(name)
            .filter(|v| v.scope == self.scope())
    }

    /// First assignment changes an absent value, including assignment of JSON null.
    /// JSON equality suppresses same-value changes within a generation.
    pub fn assign(&mut self, name: &str, payload: Json) -> Result<Option<Uuid>> {
        ensure!(!name.is_empty(), "empty value name");
        let previous = self.value(name);
        if previous.is_some_and(|v| v.payload == payload) {
            return Ok(None);
        }
        let revision = previous
            .map_or(0, |v| v.revision)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("value revision overflow"))?;
        let event_id = self.emit_event(
            EventKind::ValueChanged {
                name: name.to_owned(),
                revision,
            },
            payload.clone(),
            None,
        )?;
        self.dataflow.values.insert(
            name.to_owned(),
            Value {
                scope: self.scope(),
                revision,
                payload,
            },
        );
        Ok(Some(event_id))
    }

    /// Trusted ingress assigns identity. Equal payloads are distinct occurrences.
    /// Re-delivery must retain the existing event_id, not call publish again.
    pub fn publish(
        &mut self,
        topic: &str,
        payload: Json,
        causation_id: Option<Uuid>,
    ) -> Result<Uuid> {
        ensure!(!topic.is_empty(), "empty event topic");
        self.emit_event(
            EventKind::External {
                topic: topic.to_owned(),
            },
            payload,
            causation_id,
        )
    }

    pub(crate) fn emit_event(
        &mut self,
        kind: EventKind,
        payload: Json,
        causation_id: Option<Uuid>,
    ) -> Result<Uuid> {
        self.validate_cause(causation_id, &self.scope())?;
        let sequence = u64::try_from(self.dataflow.events.len())?
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("event sequence overflow"))?;
        let event_id = Uuid::new_v4();
        self.dataflow.events.push(Event {
            event_id,
            sequence,
            scope: self.scope(),
            causation_id,
            kind,
            payload,
        });
        Ok(event_id)
    }

    /// FIFO within the current generation. Old events remain history, never triggers.
    pub fn next_event(&self) -> Option<&Event<Json>> {
        self.dataflow
            .events
            .iter()
            .find(|e| e.scope == self.scope() && !self.dataflow.consumed.contains(&e.event_id))
    }

    /// In-memory transition. Durable callers must persist before exposing the event.
    pub fn consume(&mut self, event_id: Uuid) -> Result<Option<Event<Json>>> {
        let event = self
            .dataflow
            .events
            .iter()
            .find(|e| e.event_id == event_id)
            .ok_or_else(|| anyhow::anyhow!("unknown event"))?;
        ensure!(event.scope == self.scope(), "stale event");
        if !self.dataflow.consumed.insert(event_id) {
            return Ok(None);
        }
        Ok(Some(event.clone()))
    }

    pub(crate) fn validate_cause(&self, cause: Option<Uuid>, scope: &Scope) -> Result<()> {
        if let Some(id) = cause {
            ensure!(
                self.dataflow
                    .events
                    .iter()
                    .any(|e| e.event_id == id && &e.scope == scope),
                "unknown or stale event cause"
            );
        }
        Ok(())
    }

    pub(crate) fn validate_dataflow(&self) -> Result<()> {
        let scope = self.scope();
        let valid_scope = |s: &Scope| {
            s.workflow_run_id == scope.workflow_run_id
                && s.correlation_id == scope.correlation_id
                && s.generation <= scope.generation
        };
        let mut seen = BTreeMap::new();
        let mut terminal = BTreeSet::new();
        let mut changes: BTreeMap<(u64, &str), &Event<Json>> = BTreeMap::new();
        let mut last_generation = 0;
        for (index, event) in self.dataflow.events.iter().enumerate() {
            ensure!(
                valid_scope(&event.scope)
                    && event.sequence == index as u64 + 1
                    && event.scope.generation >= last_generation,
                "invalid event scope or sequence"
            );
            last_generation = event.scope.generation;
            if let Some(cause) = event.causation_id {
                ensure!(
                    seen.get(&cause) == Some(&event.scope),
                    "invalid event causation"
                );
            }
            ensure!(
                seen.insert(event.event_id, event.scope.clone()).is_none(),
                "duplicate event ID"
            );
            match &event.kind {
                EventKind::External { topic } => ensure!(!topic.is_empty(), "empty event topic"),
                EventKind::ValueChanged { name, revision } => {
                    ensure!(!name.is_empty(), "empty value name");
                    let previous = changes.insert((event.scope.generation, name), event);
                    let expected = match previous.map(|e| &e.kind) {
                        Some(EventKind::ValueChanged { revision, .. }) => revision.checked_add(1),
                        _ => Some(1),
                    };
                    ensure!(Some(*revision) == expected, "invalid value revision");
                    ensure!(
                        previous.is_none_or(|e| e.payload != event.payload),
                        "unchanged value event"
                    );
                }
                EventKind::InvocationTerminated {
                    invocation_id,
                    status,
                } => {
                    let record = self
                        .invocation(*invocation_id)
                        .ok_or_else(|| anyhow::anyhow!("unknown event invocation"))?;
                    ensure!(
                        *status != InvocationStatus::Pending
                            && record.status == *status
                            && record.identity.generation == event.scope.generation
                            && event.payload == Json::Null
                            && event.causation_id == record.identity.causation_id
                            && terminal.insert(*invocation_id),
                        "invalid invocation event"
                    );
                }
            }
        }
        ensure!(
            self.dataflow
                .consumed
                .iter()
                .all(|id| seen.contains_key(id)),
            "unknown consumed event"
        );
        // Every terminal invocation must have exactly one lifecycle notification.
        for record in self.invocations() {
            self.validate_cause(
                record.identity.causation_id,
                &Scope {
                    workflow_run_id: record.identity.workflow_run_id,
                    correlation_id: record.identity.correlation_id,
                    generation: record.identity.generation,
                },
            )?;
            ensure!(
                (record.status != InvocationStatus::Pending)
                    == terminal.contains(&record.identity.invocation_id),
                "missing invocation event"
            );
        }
        for (name, value) in &self.dataflow.values {
            ensure!(valid_scope(&value.scope), "invalid value scope");
            let event = changes
                .get(&(value.scope.generation, name.as_str()))
                .ok_or_else(|| anyhow::anyhow!("missing value change"))?;
            ensure!(
                matches!(&event.kind, EventKind::ValueChanged { revision, .. } if *revision == value.revision)
                    && event.payload == value.payload,
                "value checkpoint mismatch"
            );
        }
        for ((generation, name), _) in changes {
            ensure!(
                self.dataflow
                    .values
                    .get(name)
                    .is_some_and(|v| v.scope.generation >= generation),
                "missing latest value"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn restart(state: &ExecutionState) -> ExecutionState {
        ExecutionState::restore(&serde_json::to_vec(state).unwrap()).unwrap()
    }

    #[test]
    fn values_change_only_on_assignment_of_a_different_value() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let first = state
            .assign("candidate", json!({"text":"a"}))
            .unwrap()
            .unwrap();
        assert!(
            state
                .assign("candidate", json!({"text":"a"}))
                .unwrap()
                .is_none()
        );
        state.consume(first).unwrap();
        assert!(state.next_event().is_none());
        assert_eq!(state.value("candidate").unwrap().revision, 1);
        let second = state
            .assign("candidate", json!({"text":"b"}))
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        let mut restored = restart(&state);
        assert_eq!(
            restored.value("candidate").unwrap().payload,
            json!({"text":"b"})
        );
        assert_eq!(
            restored.consume(second).unwrap().unwrap().payload,
            json!({"text":"b"})
        );
        assert!(restored.consume(first).unwrap().is_none());
        assert_eq!(restored.value("candidate").unwrap().revision, 2);
    }

    #[test]
    fn equal_payload_events_remain_distinct_and_consumption_survives_replay() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let a = state.publish("timer", json!(42), None).unwrap();
        let b = state.publish("timer", json!(42), Some(a)).unwrap();
        assert_ne!(a, b);
        assert_eq!(state.next_event().unwrap().event_id, a);
        state.consume(a).unwrap();
        let mut restored = restart(&state);
        assert!(restored.consume(a).unwrap().is_none());
        assert_eq!(restored.next_event().unwrap().event_id, b);
        assert_eq!(restored.consume(b).unwrap().unwrap().causation_id, Some(a));
        assert!(restart(&restored).next_event().is_none());
        assert!(restored.value("timer").is_none());
    }

    #[test]
    fn completion_is_bound_to_one_invocation_and_is_not_a_value_assignment() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let trigger = state.publish("request", Json::Null, None).unwrap();
        state.consume(trigger).unwrap();
        let a = state.begin_caused("build", None, Some(trigger)).unwrap();
        let b = state.begin("review", None).unwrap();
        let mut forged = a.clone();
        forged.invocation_id = b.invocation_id;
        assert!(state.finish_result(&forged, Some(json!("wrong"))).is_err());
        assert!(state.next_event().is_none());
        state.finish_result(&a, Some(Json::Null)).unwrap();
        assert_eq!(
            state.invocation(b.invocation_id).unwrap().status,
            InvocationStatus::Pending
        );
        let event = state.next_event().unwrap().clone();
        assert_eq!(
            event.kind,
            EventKind::InvocationTerminated {
                invocation_id: a.invocation_id,
                status: InvocationStatus::Completed
            }
        );
        assert_eq!(event.causation_id, Some(trigger));
        assert!(state.value("build").is_none());
        state.consume(event.event_id).unwrap();
        let mut restored = restart(&state);
        assert_eq!(
            restored
                .invocation(a.invocation_id)
                .unwrap()
                .result
                .as_ref()
                .unwrap()
                .payload,
            Json::Null
        );
        assert!(restored.finish(&a, true).is_err());
        assert!(restored.next_event().is_none());
        restored.cancel(&b).unwrap();
        assert!(restored.finish(&b, true).is_err());
        assert_eq!(
            restart(&restored)
                .invocation(b.invocation_id)
                .unwrap()
                .status,
            InvocationStatus::Cancelled
        );
    }

    #[test]
    fn new_generation_cannot_read_old_values_or_deliver_old_events() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let old = state.assign("candidate", json!(1)).unwrap().unwrap();
        state.advance_generation().unwrap();
        assert!(state.value("candidate").is_none());
        assert!(state.next_event().is_none());
        assert!(state.consume(old).is_err());
        assert!(state.publish("timer", Json::Null, Some(old)).is_err());
        assert!(state.begin_caused("build", None, Some(old)).is_err());
        assert!(state.assign("candidate", json!(1)).unwrap().is_some());
        assert_eq!(restart(&state).value("candidate").unwrap().revision, 1);
    }

    #[test]
    fn restore_rejects_corrupt_event_value_and_completion_records() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let event_id = state.assign("value", json!(1)).unwrap().unwrap();
        let invocation = state.begin("call", None).unwrap();
        state
            .finish_result(&invocation, Some(json!("result")))
            .unwrap();
        let valid = serde_json::to_value(&state).unwrap();
        let reject =
            |v: Json| assert!(ExecutionState::restore(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut bad = valid.clone();
        bad["dataflow"]["events"][1]["event_id"] = json!(event_id);
        reject(bad);
        let mut bad = valid.clone();
        bad["dataflow"]["consumed"] = json!([Uuid::new_v4()]);
        reject(bad);
        let mut bad = valid.clone();
        bad["dataflow"]["values"]["value"]["payload"] = json!(2);
        reject(bad);
        let mut bad = valid.clone();
        bad["dataflow"]["events"][0]["causation_id"] = json!(event_id);
        reject(bad);
        let mut bad = valid.clone();
        bad["dataflow"]["events"][0]["scope"]["correlation_id"] = json!(Uuid::new_v4());
        reject(bad);
        let mut bad = valid.clone();
        bad["dataflow"]["events"].as_array_mut().unwrap().pop();
        reject(bad);
        let mut bad = valid.clone();
        bad["records"][invocation.invocation_id.to_string()]["result"] = Json::Null;
        reject(bad);
        let mut bad = valid;
        bad["version"] = json!(1);
        reject(bad);
    }
}
