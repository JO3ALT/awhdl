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
        let config = Self {
            root,
            runtime,
            models,
            routes,
            mcp,
        };
        config.validate()?;
        Ok(config)
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
            match route.kind.as_str() {
                "model" => {
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default)]
    pub match_all: Vec<String>,
    pub steps: Vec<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAdapterKind {
    Codex,
    ClaudeReview,
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
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        ProjectConfig::load(root).unwrap();
    }

    #[test]
    fn selects_the_open_data_population_workflow() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let config = ProjectConfig::load(root).unwrap();
        let selected = config
            .routes
            .select_workflow("World Bank SP.POP.TOTL population_assessment_from_data")
            .map(|(name, _)| name);
        assert_eq!(selected, Some("open_data_population"));
    }
}
