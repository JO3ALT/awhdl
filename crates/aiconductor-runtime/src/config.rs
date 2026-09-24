use crate::capability::{Authorization, CapabilityPolicy, CapabilityRequest};
use crate::completion::CompletionPolicy;
use crate::effect::EffectClass;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub root: PathBuf,
    pub runtime: RuntimeConfig,
    pub models: ModelInventory,
    pub routes: RoutingConfig,
    pub mcp: McpInventory,
    pub capabilities: CapabilityPolicy,
    /// Use only the default orchestrator as controller, without fallbacks.
    /// Set by an explicit controller override so comparisons stay unmixed.
    pub controller_strict: bool,
}

impl ProjectConfig {
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .with_context(|| format!("project root does not exist: {}", root.as_ref().display()))?;
        let runtime = load_toml::<RuntimeConfig>(&root.join("config/runtime.toml"))?;
        let models = load_toml::<ModelInventory>(&root.join("config/llm-hosts.toml"))?;
        let routes = load_toml::<RoutingConfig>(&root.join("config/routing-defaults.toml"))?;
        let mcp = load_toml::<McpInventory>(&root.join("config/mcp-servers.toml"))?;
        let capabilities = load_toml::<CapabilityPolicy>(&root.join("config/capabilities.toml"))?;
        let config = Self {
            root,
            runtime,
            models,
            routes,
            mcp,
            capabilities,
            controller_strict: false,
        };
        config.validate()?;
        Ok(config)
    }

    /// Authorize a route's own static needs and cap each grant's class at the
    /// route's declared class, so the route-level retry/approval rules stay sound.
    pub fn authorize_route(
        &self,
        name: &str,
        request: &CapabilityRequest,
    ) -> Result<Authorization> {
        let route = self
            .action(name)
            .with_context(|| format!("unknown route {name}"))?;
        let authorization = self.capabilities.authorize(name, request)?;
        if authorization.effect_class.rank() > route.effective_class().rank() {
            bail!(
                "capability {} exceeds the effect class of route {name}",
                authorization.capability_id
            );
        }
        Ok(authorization)
    }

    fn validate_capabilities(&self) -> Result<()> {
        self.capabilities.validate()?;
        for grant in &self.capabilities.grants {
            for subject in &grant.subjects {
                if !self.routes.actions.contains_key(subject) {
                    bail!("capability {} names unknown route {subject}", grant.id);
                }
            }
        }
        self.authorize_route(
            &self.routes.default_action,
            &CapabilityRequest::new(
                "model.chat",
                format!("model:{}", self.models.default_orchestrator),
            ),
        )?;
        for (name, route) in &self.routes.actions {
            let requests = match route.kind.as_str() {
                "model" => route
                    .model
                    .iter()
                    .chain(&route.fallback)
                    .map(|model| CapabilityRequest::new("model.chat", format!("model:{model}")))
                    .collect::<Vec<_>>(),
                "mcp" => {
                    let server = route.server.as_deref().unwrap_or_default();
                    let mut requests = route
                        .tool
                        .iter()
                        .chain(&route.preferred_tools)
                        .map(|tool| {
                            CapabilityRequest::new("mcp.call", format!("mcp:{server}/{tool}"))
                        })
                        .collect::<Vec<_>>();
                    if route.adapter == Some(McpAdapterKind::Codex) {
                        requests.extend(crate::adapters::codex_capability_requests(route)?);
                    }
                    requests
                }
                _ => Vec::new(),
            };
            for request in requests {
                self.authorize_route(name, &request)
                    .with_context(|| format!("route {name} is not fully authorized"))?;
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.runtime.schema_version != 1
            || self.models.schema_version != 1
            || self.routes.schema_version != 1
            || self.mcp.schema_version != 1
        {
            bail!("unsupported configuration schema version");
        }
        if !self
            .routes
            .actions
            .contains_key(&self.routes.default_action)
        {
            bail!(
                "default action is not defined: {}",
                self.routes.default_action
            );
        }
        for (name, route) in &self.routes.actions {
            if route.max_attempts == 0 {
                bail!("route {name} max_attempts must be greater than zero");
            }
            for dependency in &route.depends_on {
                if dependency == name {
                    bail!("route {name} cannot depend on itself");
                }
                if !self.routes.actions.contains_key(dependency) {
                    bail!("route {name} depends on unknown action {dependency}");
                }
            }
            if route
                .idempotency_argument
                .as_ref()
                .is_some_and(String::is_empty)
            {
                bail!("route {name} has an empty idempotency argument");
            }
            if matches!(
                route.effective_class(),
                EffectClass::ExternalWrite | EffectClass::Destructive
            ) && route.idempotency_argument.is_none()
                && !route.manual_reconciliation
            {
                bail!("route {name} needs provider idempotency or manual reconciliation");
            }
            route
                .route_location()
                .with_context(|| format!("route {name}"))?;
            if route.human_approval.is_some() && !route.effective_class().is_write() {
                bail!("route {name} sets human_approval but is not a write");
            }
            if route.effective_class() == EffectClass::Destructive
                && !route.requires_human_approval()
            {
                bail!("destructive route {name} cannot opt out of human approval");
            }
            match route.kind.as_str() {
                "model" => {
                    if route.effective_class().is_write() {
                        bail!("model route {name} cannot use effectful MCP dispatch");
                    }
                    let model = route
                        .model
                        .as_deref()
                        .with_context(|| format!("model route {name} has no model"))?;
                    if self.profile(model).is_none() {
                        bail!("route {name} references unknown model profile {model}");
                    }
                }
                "mcp" => {
                    let server = route
                        .server
                        .as_deref()
                        .with_context(|| format!("MCP route {name} has no server"))?;
                    if !self.mcp.servers.contains_key(server) {
                        bail!("route {name} references unknown MCP server {server}");
                    }
                }
                other => bail!("route {name} has unsupported kind {other}"),
            }
        }
        for (name, workflow) in &self.routes.workflows {
            if workflow.match_all.is_empty() || workflow.steps.is_empty() {
                bail!("workflow {name} must define match_all and steps");
            }
            for step in &workflow.steps {
                if step == &self.routes.default_action {
                    bail!("workflow {name} cannot contain the controller action");
                }
                if !self.routes.actions.contains_key(step) {
                    bail!("workflow {name} references unknown action {step}");
                }
            }
        }
        if let Some(approval) = &self.runtime.approval {
            if !self.mcp.servers.contains_key(&approval.server) {
                bail!("approval server {} is not configured", approval.server);
            }
            if !(1..=86_400).contains(&approval.ttl_sec) || approval.poll_ms == 0 {
                bail!("approval ttl_sec must be 1..=86400 and poll_ms positive");
            }
            // The approval channel is trusted: no agent route may reach it.
            if let Some((name, _)) = self
                .routes
                .actions
                .iter()
                .find(|(_, route)| route.server.as_deref() == Some(approval.server.as_str()))
            {
                bail!("route {name} must not use the approval server");
            }
        }
        self.validate_completion()?;
        self.validate_capabilities()?;
        self.validate_decider()
    }

    fn is_model_route(&self, action: &str) -> bool {
        self.action(action)
            .is_some_and(|route| route.kind == "model")
    }

    /// Hard conditions are deterministic and reference known actions; a
    /// workflow with an external or destructive step must be safety-critical.
    fn validate_completion(&self) -> Result<()> {
        let policies = std::iter::once(("<default>", &self.routes.completion, None)).chain(
            self.routes
                .workflows
                .iter()
                .map(|(name, workflow)| (name.as_str(), &workflow.completion, Some(workflow))),
        );
        for (name, policy, workflow) in policies {
            policy
                .validate(|action| self.is_model_route(action))
                .with_context(|| format!("completion policy of {name}"))?;
            for condition in policy
                .hard_conditions()?
                .iter()
                .chain(&policy.soft_conditions()?)
            {
                if let Some(action) = condition.action()
                    && !self.routes.actions.contains_key(action)
                {
                    bail!("completion policy of {name} references unknown action {action}");
                }
            }
            if let Some(workflow) = workflow {
                let external = workflow.steps.iter().any(|step| {
                    self.action(step).is_some_and(|route| {
                        route.effective_class().rank() >= EffectClass::ExternalWrite.rank()
                    })
                });
                if external && !policy.safety_critical {
                    bail!("workflow {name} has an external write step and must be safety_critical");
                }
            }
        }
        Ok(())
    }

    pub fn profile(&self, id: &str) -> Option<&LaunchProfile> {
        self.models
            .launch_profiles
            .iter()
            .find(|item| item.id == id)
    }

    pub fn action(&self, id: &str) -> Option<&ActionConfig> {
        self.routes.actions.get(id)
    }

    /// Run with this launch profile as the only controller model.
    pub fn override_controller(&mut self, profile: &str) -> Result<()> {
        if self.profile(profile).is_none() {
            bail!("unknown controller profile: {profile}");
        }
        self.models.default_orchestrator = profile.to_owned();
        self.controller_strict = true;
        self.validate()
            .with_context(|| format!("controller {profile} is not usable"))
    }

    /// Enable or disable the configured decider for this run.
    pub fn set_decider_enabled(&mut self, enabled: bool) -> Result<()> {
        let decider = self
            .runtime
            .decider
            .as_mut()
            .context("no [decider] is configured")?;
        decider.enabled = enabled;
        self.validate()
    }

    /// Whether the planner may choose cloud routes for this instruction.
    pub fn cloud_opted_in(&self, prompt: &str) -> bool {
        self.routes
            .policy
            .cloud_opt_in_marker
            .as_deref()
            .is_some_and(|marker| !marker.is_empty() && prompt.contains(marker))
    }

    pub fn is_cloud_route(&self, id: &str) -> bool {
        self.action(id).is_some_and(|route| {
            route
                .route_location()
                .is_ok_and(|location| location == awhdl_checker::Location::Cloud)
        })
    }

    pub fn resolve_path(&self, value: &str) -> PathBuf {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

fn load_toml<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read configuration: {}", path.display()))?;
    toml::from_str(&text)
        .with_context(|| format!("failed to parse configuration: {}", path.display()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub run_root: String,
    pub ssh: SshConfig,
    #[serde(rename = "loop")]
    pub loop_limits: LoopLimits,
    pub planner: PlannerConfig,
    /// Without an approval adapter, approval-bound writes fail closed.
    #[serde(default)]
    pub approval: Option<ApprovalAdapterConfig>,
    /// Optional decision model that selects the controller's next action.
    #[serde(default)]
    pub decider: Option<DeciderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeciderConfig {
    pub enabled: bool,
    /// Capability resource is `decider:<id>`.
    pub id: String,
    /// `POST` endpoint of a System One style `/v1/decisions` service.
    pub endpoint: String,
    /// `local` or `cloud`; a cloud decider receives the compact run state.
    pub location: String,
    /// Below this, the LLM planner makes the whole decision instead.
    pub min_confidence: f64,
    /// Upper bound of the compact state sent to the model.
    pub max_state_chars: usize,
    #[serde(default = "default_decider_timeout")]
    pub timeout_sec: u64,
}

fn default_decider_timeout() -> u64 {
    10
}

impl ProjectConfig {
    /// The configured decision model, if one is enabled.
    pub fn active_decider(&self) -> Option<&DeciderConfig> {
        self.runtime
            .decider
            .as_ref()
            .filter(|decider| decider.enabled)
    }

    fn validate_decider(&self) -> Result<()> {
        let Some(decider) = self.active_decider() else {
            return Ok(());
        };
        if !(decider.endpoint.starts_with("http://") || decider.endpoint.starts_with("https://")) {
            bail!("decider endpoint must be an http(s) URL");
        }
        if !matches!(decider.location.as_str(), "local" | "cloud") {
            bail!("decider location must be local or cloud");
        }
        if !(0.0..=1.0).contains(&decider.min_confidence) {
            bail!("decider min_confidence must be within 0..=1");
        }
        if decider.max_state_chars < 200 || decider.timeout_sec == 0 {
            bail!("decider needs max_state_chars >= 200 and a positive timeout_sec");
        }
        self.authorize_route(
            &self.routes.default_action,
            &CapabilityRequest::new("model.decide", format!("decider:{}", decider.id)),
        )
        .context("the enabled decider is not granted to the controller route")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalAdapterConfig {
    /// MCP server running HumanPort in approver mode.
    pub server: String,
    #[serde(default = "default_approval_ttl")]
    pub ttl_sec: i64,
    #[serde(default = "default_approval_poll")]
    pub poll_ms: u64,
}

fn default_approval_ttl() -> i64 {
    600
}

fn default_approval_poll() -> u64 {
    500
}

#[derive(Debug, Clone, Deserialize)]
pub struct SshConfig {
    pub binary: String,
    pub identity_file: String,
    pub connect_timeout_sec: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoopLimits {
    pub max_iterations: u32,
    pub max_wall_time_sec: u64,
    pub max_model_calls: u32,
    pub max_mcp_calls: u32,
    pub max_model_switches: u32,
    pub max_network_requests: u32,
    pub max_download_bytes: u64,
    pub model_start_timeout_sec: u64,
    pub device_timeout_sec: u64,
    pub planner_retries: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlannerConfig {
    pub temperature: f32,
    pub max_observation_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInventory {
    pub schema_version: u32,
    pub default_orchestrator: String,
    pub hosts: BTreeMap<String, HostConfig>,
    pub launch_profiles: Vec<LaunchProfile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostConfig {
    pub ssh_target: String,
    #[serde(default)]
    pub tunnel: Option<TunnelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TunnelConfig {
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchProfile {
    pub id: String,
    pub host: String,
    pub model: String,
    pub launch_script: String,
    pub status: String,
    pub api_base: String,
    #[serde(default)]
    pub modality: String,
    #[serde(default)]
    pub exclusive_group: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub planner_reasoning_effort: Option<String>,
}

impl LaunchProfile {
    pub fn model_name(&self) -> Result<&str> {
        Path::new(&self.model)
            .file_name()
            .and_then(|name| name.to_str())
            .context("model path has no UTF-8 file name")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutingConfig {
    pub schema_version: u32,
    pub default_action: String,
    pub actions: BTreeMap<String, ActionConfig>,
    #[serde(default)]
    pub presentation_pipeline: BTreeMap<String, String>,
    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowConfig>,
    /// Completion policy for runs that match no workflow.
    #[serde(default)]
    pub completion: CompletionPolicy,
    #[serde(default)]
    pub policy: RoutingPolicy,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RoutingPolicy {
    /// Cloud routes send data off this network, so the planner may choose them
    /// only when the operator's instruction contains this marker. Without a
    /// marker they are reachable only through configured workflows.
    #[serde(default)]
    pub cloud_opt_in_marker: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default)]
    pub match_all: Vec<String>,
    pub steps: Vec<String>,
    #[serde(default)]
    pub completion: CompletionPolicy,
}

impl RoutingConfig {
    pub fn select_workflow(&self, prompt: &str) -> Option<(&str, &WorkflowConfig)> {
        let prompt = prompt.to_lowercase();
        self.workflows.iter().find_map(|(name, workflow)| {
            (!workflow.match_all.is_empty()
                && workflow
                    .match_all
                    .iter()
                    .all(|pattern| prompt.contains(&pattern.to_lowercase())))
            .then_some((name.as_str(), workflow))
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionConfig {
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub effect_class: Option<EffectClass>,
    #[serde(default)]
    pub idempotency_argument: Option<String>,
    #[serde(default)]
    pub manual_reconciliation: bool,
    /// Action-bound human approval for write Effects. Distinct from Codex's
    /// `approval_policy`, which is forwarded to the Codex CLI.
    #[serde(default)]
    pub human_approval: Option<HumanApproval>,
    /// Where the route's data goes: `local` (default) or `cloud`. AWHDL devices
    /// bound to the route must declare the same location.
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback: Vec<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub preferred_tools: Vec<String>,
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub approval_policy: Option<String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub adapter: Option<McpAdapterKind>,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub repeatable: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl ActionConfig {
    pub fn effective_class(&self) -> EffectClass {
        self.effect_class.unwrap_or(if self.kind == "model" {
            EffectClass::Pure
        } else {
            EffectClass::ExternalWrite
        })
    }

    pub fn route_location(&self) -> Result<awhdl_checker::Location> {
        match self.location.as_deref() {
            None => Ok(awhdl_checker::Location::Local),
            Some(name) => awhdl_checker::parse_location(name)
                .with_context(|| format!("unsupported route location {name}")),
        }
    }

    /// External and destructive writes default to required approval.
    pub fn requires_human_approval(&self) -> bool {
        self.requires_human_approval_for(self.effective_class())
    }

    /// Approval for one call, whose class comes from the authorizing capability.
    pub fn requires_human_approval_for(&self, class: EffectClass) -> bool {
        match self.human_approval {
            Some(setting) => setting == HumanApproval::Required,
            None => matches!(class, EffectClass::ExternalWrite | EffectClass::Destructive),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanApproval {
    Required,
    NotRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAdapterKind {
    Codex,
    Lean,
    Prolog,
    Matlab,
    Passthrough,
}

fn default_max_attempts() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpInventory {
    pub schema_version: u32,
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub enabled: bool,
    pub transport: String,
    pub command: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_configuration_is_consistent() {
        let fixture = crate::test_support::project_root();
        ProjectConfig::load(&fixture).unwrap();
        // On a development machine, also validate the real deployment and keep
        // the fixture's routes and grants identical to it.
        if let Some(root) = crate::test_support::deployment_root() {
            ProjectConfig::load(&root).unwrap();
            for file in ["routing-defaults.toml", "capabilities.toml"] {
                assert_eq!(
                    fs::read_to_string(root.join("config").join(file)).unwrap(),
                    fs::read_to_string(fixture.join("config").join(file)).unwrap(),
                    "test fixture {file} is out of sync with the deployment"
                );
            }
        }
    }

    #[test]
    fn human_approval_defaults_and_opt_outs_are_validated() {
        let root = crate::test_support::project_root();
        let mut config = ProjectConfig::load(root).unwrap();
        let route = |config: &ProjectConfig, name: &str| config.action(name).unwrap().clone();
        // Every external write, including Codex open-data acquisition, needs approval.
        assert!(route(&config, "open_data_acquisition").requires_human_approval());
        assert!(!route(&config, "calculation_graphing").requires_human_approval());
        let action = config
            .routes
            .actions
            .get_mut("open_data_acquisition")
            .unwrap();
        // An explicit opt-out remains possible for external writes...
        action.human_approval = Some(HumanApproval::NotRequired);
        assert!(!action.requires_human_approval());
        assert!(config.validate().is_ok());
        let action = config
            .routes
            .actions
            .get_mut("open_data_acquisition")
            .unwrap();
        // ...but never for destructive ones.
        action.effect_class = Some(EffectClass::Destructive);
        action.human_approval = Some(HumanApproval::NotRequired);
        assert!(config.validate().is_err());
        let action = config.routes.actions.get_mut("logic_rules").unwrap();
        action.human_approval = Some(HumanApproval::Required);
        config
            .routes
            .actions
            .get_mut("open_data_acquisition")
            .unwrap()
            .human_approval = None;
        assert!(config.validate().is_err());
    }

    #[test]
    fn capabilities_unify_tool_file_sandbox_and_model_policy() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(&root).unwrap();
        let mcp = |subject: &str, resource: &str| {
            config.authorize_route(subject, &CapabilityRequest::new("mcp.call", resource))
        };
        // Tool-level classes inside one route.
        assert_eq!(
            mcp("table_analysis", "mcp:restricted_kdb/get_interpreter_state")
                .unwrap()
                .effect_class,
            EffectClass::Read
        );
        assert_eq!(
            mcp("table_analysis", "mcp:restricted_kdb/save_csv")
                .unwrap()
                .effect_class,
            EffectClass::LocalWrite
        );
        assert!(mcp("table_analysis", "mcp:restricted_kdb/drop_everything").is_err());
        assert!(mcp("calculation_graphing", "mcp:restricted_kdb/run_q").is_err());
        assert!(
            config
                .authorize_route(
                    "logic_rules",
                    &CapabilityRequest::new("file.read", "file:.runtime/secrets/runtime.toml"),
                )
                .is_err()
        );
        assert!(
            config
                .authorize_route(
                    "deep_reasoning",
                    &CapabilityRequest::new("network.connect", "host:*"),
                )
                .is_err()
        );
        assert!(
            config
                .authorize_route(
                    "vision",
                    &CapabilityRequest::new("model.chat", "model:qwen3.8-27b-q5km"),
                )
                .is_err()
        );
        let open_data = config.action("open_data_acquisition").unwrap();
        let actions = crate::adapters::codex_capability_requests(open_data)
            .unwrap()
            .into_iter()
            .map(|request| request.action)
            .collect::<Vec<_>>();
        assert_eq!(actions, ["sandbox.full_access", "network.connect"]);
    }

    #[test]
    fn controller_override_is_strict_and_must_be_granted() {
        let root = crate::test_support::project_root();
        let mut config = ProjectConfig::load(&root).unwrap();
        assert!(!config.controller_strict);
        assert!(config.override_controller("no-such-profile").is_err());
        // A model only granted to japanese_polishing cannot become the controller.
        let mut polishing = config.clone();
        assert!(
            polishing
                .override_controller("llm-jp-4-8b-thinking-bf16")
                .is_err()
        );
        config.override_controller("qwen3vl-8b-rocm").unwrap();
        assert_eq!(config.models.default_orchestrator, "qwen3vl-8b-rocm");
        assert!(config.controller_strict);
    }

    #[test]
    fn cloud_routes_need_the_opt_in_marker() {
        let root = crate::test_support::project_root();
        let mut config = ProjectConfig::load(&root).unwrap();
        assert!(config.is_cloud_route("deep_reasoning"));
        assert!(config.is_cloud_route("open_data_acquisition"));
        assert!(!config.is_cloud_route("logic_rules"));
        assert!(!config.is_cloud_route("unknown_action"));
        assert!(!config.cloud_opted_in("Prologで確認してください"));
        assert!(config.cloud_opted_in("Codexでも調べてください AIC_ALLOW_CLOUD"));
        // Without a marker, only workflows reach cloud routes.
        config.routes.policy.cloud_opt_in_marker = None;
        assert!(!config.cloud_opted_in("AIC_ALLOW_CLOUD"));
        config.routes.policy.cloud_opt_in_marker = Some(String::new());
        assert!(!config.cloud_opted_in("anything"));
    }

    #[test]
    fn an_enabled_decider_must_be_granted_and_well_formed() {
        let root = crate::test_support::project_root();
        let mut config = ProjectConfig::load(&root).unwrap();
        config.runtime.decider = Some(DeciderConfig {
            enabled: true,
            id: "jev".to_owned(),
            endpoint: "http://127.0.0.1:18090/v1/decisions".to_owned(),
            location: "local".to_owned(),
            min_confidence: 0.6,
            max_state_chars: 3000,
            timeout_sec: 10,
        });
        assert!(
            config.validate().is_err(),
            "ungranted decider must be rejected"
        );
        config
            .capabilities
            .grants
            .push(crate::capability::Capability {
                id: "decider".to_owned(),
                subjects: vec![config.routes.default_action.clone()],
                action: "model.decide".to_owned(),
                resource: "decider:jev".to_owned(),
                constraints: BTreeMap::new(),
                effect_class: EffectClass::Pure,
                revoked: false,
            });
        config.validate().unwrap();
        let decider = config.runtime.decider.as_mut().unwrap();
        decider.min_confidence = 1.5;
        assert!(config.validate().is_err());
        let decider = config.runtime.decider.as_mut().unwrap();
        decider.min_confidence = 0.6;
        decider.location = "elsewhere".to_owned();
        assert!(config.validate().is_err());
        // A disabled decider is ignored entirely.
        let decider = config.runtime.decider.as_mut().unwrap();
        decider.enabled = false;
        config.capabilities.grants.pop();
        config.validate().unwrap();
        assert!(config.active_decider().is_none());
    }

    #[test]
    fn capability_policy_changes_are_validated_against_routes() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(&root).unwrap();
        fn grant<'a>(
            config: &'a mut ProjectConfig,
            id: &str,
        ) -> &'a mut crate::capability::Capability {
            let index = config
                .capabilities
                .grants
                .iter()
                .position(|grant| grant.id == id)
                .unwrap();
            &mut config.capabilities.grants[index]
        }
        // A grant above the route's class ceiling is rejected.
        let mut escalated = config.clone();
        grant(&mut escalated, "lean_check").effect_class = EffectClass::ExternalWrite;
        assert!(escalated.validate().is_err());
        // Revoking a needed grant leaves the route unauthorized.
        let mut revoked = config.clone();
        revoked.capabilities.revoke("prolog_query").unwrap();
        assert!(revoked.validate().is_err());
        let mut unknown = config.clone();
        grant(&mut unknown, "lean_check")
            .subjects
            .push("ghost".to_owned());
        assert!(unknown.validate().is_err());
        // A scope change produces a different handle, invalidating bound approvals.
        let before = config
            .authorize_route(
                "calculation_graphing",
                &CapabilityRequest::new("mcp.call", "mcp:matlab_r2026a/evaluate_matlab_code"),
            )
            .unwrap();
        let mut narrowed = config.clone();
        grant(&mut narrowed, "matlab_execute").resource = "mcp:matlab_r2026a/*_matlab_*".to_owned();
        let after = narrowed
            .authorize_route(
                "calculation_graphing",
                &CapabilityRequest::new("mcp.call", "mcp:matlab_r2026a/evaluate_matlab_code"),
            )
            .unwrap();
        assert_eq!(before.capability_id, after.capability_id);
        assert_ne!(before.handle, after.handle);
    }

    #[test]
    fn completion_policies_are_statically_checked() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(&root).unwrap();
        let open_data = &config.routes.workflows["open_data_population"].completion;
        assert!(open_data.safety_critical && !open_data.hard.is_empty());
        let edit = |change: &dyn Fn(&mut ProjectConfig)| {
            let mut edited = config.clone();
            change(&mut edited);
            edited.validate()
        };
        fn workflow(config: &mut ProjectConfig) -> &mut CompletionPolicy {
            &mut config
                .routes
                .workflows
                .get_mut("open_data_population")
                .unwrap()
                .completion
        }
        // External write step without a safety-critical policy.
        assert!(edit(&|c| workflow(c).safety_critical = false).is_err());
        // Model output or planner claims cannot be hard conditions.
        assert!(edit(&|c| workflow(c).hard.push("planner_claim".to_owned())).is_err());
        assert!(edit(&|c| workflow(c).hard.push("succeeded:initial_coding".to_owned())).is_err());
        assert!(edit(&|c| workflow(c).hard.push("succeeded:ghost".to_owned())).is_err());
        assert!(edit(&|c| workflow(c).soft.push("succeeded:initial_coding".to_owned())).is_ok());
    }

    #[test]
    fn approval_server_is_unreachable_from_routes() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(&root).unwrap();
        let approval = config
            .runtime
            .approval
            .clone()
            .expect("approval adapter configured");
        let mut exposed = config.clone();
        exposed
            .routes
            .actions
            .get_mut("text_filtering")
            .unwrap()
            .server = Some(approval.server);
        let error = format!("{:#}", exposed.validate().unwrap_err());
        assert!(
            error.contains("must not use the approval server"),
            "{error}"
        );
        let mut unknown_location = config.clone();
        unknown_location
            .routes
            .actions
            .get_mut("text_filtering")
            .unwrap()
            .location = Some("sandbox".to_owned());
        assert!(unknown_location.validate().is_err());
    }

    #[test]
    fn selects_the_open_data_population_workflow() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let selected = config
            .routes
            .select_workflow("World Bank SP.POP.TOTL population_assessment_from_data")
            .map(|(name, _)| name);
        assert_eq!(selected, Some("open_data_population"));
    }
}
