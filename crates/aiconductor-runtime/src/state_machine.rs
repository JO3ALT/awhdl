use crate::config::ActionConfig;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionState {
    pub action: String,
    pub status: ActionStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub repeatable: bool,
    pub depends_on: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    #[error("unknown action: {0}")]
    UnknownAction(String),
    #[error("action is already running: {0}")]
    AlreadyRunning(String),
    #[error("action already succeeded and is not repeatable: {0}")]
    AlreadySucceeded(String),
    #[error("action exhausted its {max_attempts} attempts: {action}")]
    AttemptsExhausted { action: String, max_attempts: u32 },
    #[error("action {action} is blocked by incomplete dependencies: {dependencies:?}")]
    Blocked {
        action: String,
        dependencies: Vec<String>,
    },
}

pub struct ActionStateMachine {
    states: BTreeMap<String, ActionState>,
    workflow_steps: Option<Vec<String>>,
}

impl ActionStateMachine {
    pub fn new(
        actions: &BTreeMap<String, ActionConfig>,
        controller: &str,
        workflow_steps: Option<&[String]>,
    ) -> Self {
        let states = actions
            .iter()
            .filter(|(name, _)| name.as_str() != controller)
            .map(|(name, config)| {
                let mut depends_on = config.depends_on.clone();
                if let Some(steps) = workflow_steps
                    && let Some(position) = steps.iter().position(|step| step == name)
                    && position > 0
                    && !depends_on.contains(&steps[position - 1])
                {
                    depends_on.push(steps[position - 1].clone());
                }
                (
                    name.clone(),
                    ActionState {
                        action: name.clone(),
                        status: ActionStatus::Pending,
                        attempts: 0,
                        max_attempts: config.max_attempts,
                        repeatable: config.repeatable,
                        depends_on,
                        last_error: None,
                    },
                )
            })
            .collect();
        Self {
            states,
            workflow_steps: workflow_steps.map(<[String]>::to_vec),
        }
    }

    pub fn start(&mut self, action: &str) -> Result<u32, TransitionError> {
        let dependencies = self
            .states
            .get(action)
            .ok_or_else(|| TransitionError::UnknownAction(action.to_owned()))?
            .depends_on
            .iter()
            .filter(|dependency| {
                self.states
                    .get(*dependency)
                    .is_none_or(|state| state.status != ActionStatus::Succeeded)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !dependencies.is_empty() {
            return Err(TransitionError::Blocked {
                action: action.to_owned(),
                dependencies,
            });
        }

        let state = self
            .states
            .get_mut(action)
            .ok_or_else(|| TransitionError::UnknownAction(action.to_owned()))?;
        match state.status {
            ActionStatus::Running => {
                return Err(TransitionError::AlreadyRunning(action.to_owned()));
            }
            ActionStatus::Succeeded if !state.repeatable => {
                return Err(TransitionError::AlreadySucceeded(action.to_owned()));
            }
            _ => {}
        }
        if state.attempts >= state.max_attempts {
            return Err(TransitionError::AttemptsExhausted {
                action: action.to_owned(),
                max_attempts: state.max_attempts,
            });
        }
        state.attempts += 1;
        state.status = ActionStatus::Running;
        state.last_error = None;
        Ok(state.attempts)
    }

    pub fn succeed(&mut self, action: &str) -> Result<()> {
        self.transition_from_running(action, ActionStatus::Succeeded, None)
    }

    pub fn fail(&mut self, action: &str, error: String) -> Result<()> {
        self.transition_from_running(action, ActionStatus::Failed, Some(error))
    }

    pub fn snapshot(&self) -> Vec<ActionState> {
        self.states.values().cloned().collect()
    }

    pub fn contains(&self, action: &str) -> bool {
        self.states.contains_key(action)
    }

    pub fn eligible_actions(&self) -> Vec<String> {
        self.states
            .values()
            .filter(|state| {
                self.workflow_steps
                    .as_ref()
                    .is_none_or(|steps| steps.iter().any(|action| action == &state.action))
            })
            .filter(|state| {
                matches!(state.status, ActionStatus::Pending | ActionStatus::Failed)
                    && state.attempts < state.max_attempts
                    && state.depends_on.iter().all(|dependency| {
                        self.states
                            .get(dependency)
                            .is_some_and(|value| value.status == ActionStatus::Succeeded)
                    })
            })
            .map(|state| state.action.clone())
            .collect()
    }

    pub fn next_workflow_action(&self) -> Result<Option<String>> {
        let Some(steps) = &self.workflow_steps else {
            return Ok(None);
        };
        for action in steps {
            let state = self
                .states
                .get(action)
                .with_context(|| format!("workflow state is missing: {action}"))?;
            if state.status == ActionStatus::Succeeded {
                continue;
            }
            if state.status == ActionStatus::Running {
                bail!("workflow action is already running: {action}");
            }
            if state.attempts >= state.max_attempts {
                bail!("workflow action exhausted retries: {action}");
            }
            let blocked = state.depends_on.iter().any(|dependency| {
                self.states
                    .get(dependency)
                    .is_none_or(|value| value.status != ActionStatus::Succeeded)
            });
            if blocked {
                bail!("workflow action has incomplete dependencies: {action}");
            }
            return Ok(Some(action.clone()));
        }
        Ok(None)
    }

    pub fn succeeded_actions(&self) -> Vec<String> {
        self.states
            .values()
            .filter(|state| state.status == ActionStatus::Succeeded)
            .map(|state| state.action.clone())
            .collect()
    }

    fn transition_from_running(
        &mut self,
        action: &str,
        status: ActionStatus,
        error: Option<String>,
    ) -> Result<()> {
        let state = self
            .states
            .get_mut(action)
            .ok_or_else(|| anyhow::anyhow!("unknown action: {action}"))?;
        if state.status != ActionStatus::Running {
            bail!("action {action} is not running");
        }
        state.status = status;
        state.last_error = error;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(depends_on: Vec<String>) -> ActionConfig {
        ActionConfig {
            description: String::new(),
            kind: "mcp".to_owned(),
            model: None,
            fallback: Vec::new(),
            server: None,
            tool: None,
            preferred_tools: Vec::new(),
            sandbox: None,
            approval_policy: None,
            workdir: None,
            network_access: false,
            adapter: None,
            max_attempts: 2,
            repeatable: false,
            depends_on,
        }
    }

    #[test]
    fn enforces_dependencies_and_single_success() {
        let mut actions = BTreeMap::new();
        actions.insert("first".to_owned(), action(Vec::new()));
        actions.insert("second".to_owned(), action(vec!["first".to_owned()]));
        let mut machine = ActionStateMachine::new(&actions, "loop", None);

        assert!(matches!(
            machine.start("second"),
            Err(TransitionError::Blocked { .. })
        ));
        assert_eq!(machine.start("first").unwrap(), 1);
        machine.succeed("first").unwrap();
        assert_eq!(
            machine.start("first"),
            Err(TransitionError::AlreadySucceeded("first".to_owned()))
        );
        assert_eq!(machine.start("second").unwrap(), 1);
    }

    #[test]
    fn enforces_attempt_limit() {
        let mut actions = BTreeMap::new();
        actions.insert("work".to_owned(), action(Vec::new()));
        let mut machine = ActionStateMachine::new(&actions, "loop", None);

        machine.start("work").unwrap();
        machine.fail("work", "one".to_owned()).unwrap();
        machine.start("work").unwrap();
        machine.fail("work", "two".to_owned()).unwrap();
        assert!(matches!(
            machine.start("work"),
            Err(TransitionError::AttemptsExhausted { .. })
        ));
    }

    #[test]
    fn advances_a_typed_workflow_in_declared_order() {
        let mut actions = BTreeMap::new();
        actions.insert("first".to_owned(), action(Vec::new()));
        actions.insert("second".to_owned(), action(Vec::new()));
        actions.insert("unrelated".to_owned(), action(Vec::new()));
        let steps = vec!["first".to_owned(), "second".to_owned()];
        let mut machine = ActionStateMachine::new(&actions, "loop", Some(&steps));

        assert!(machine.contains("unrelated"));
        assert_eq!(
            machine.next_workflow_action().unwrap().as_deref(),
            Some("first")
        );
        machine.start("first").unwrap();
        machine.succeed("first").unwrap();
        assert_eq!(
            machine.next_workflow_action().unwrap().as_deref(),
            Some("second")
        );
        machine.start("second").unwrap();
        machine.succeed("second").unwrap();
        assert_eq!(machine.next_workflow_action().unwrap(), None);
    }
}
