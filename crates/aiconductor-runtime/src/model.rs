use crate::audit::RunStore;
use crate::budget::Budget;
use crate::config::{HostConfig, LaunchProfile, ProjectConfig, TunnelConfig};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::{Instant, sleep, timeout};

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
    #[serde(default)]
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    model: String,
}

impl ModelEntry {
    fn names(&self) -> impl Iterator<Item = &str> {
        [self.id.as_str(), self.name.as_str(), self.model.as_str()]
            .into_iter()
            .filter(|item| !item.is_empty())
    }
}

pub struct ModelManager {
    client: reqwest::Client,
}

impl ModelManager {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self { client })
    }

    pub async fn ensure(
        &self,
        profile_id: &str,
        config: &ProjectConfig,
        budget: &mut Budget,
        audit: &mut RunStore,
    ) -> Result<LaunchProfile> {
        let profile = config
            .profile(profile_id)
            .with_context(|| format!("unknown launch profile: {profile_id}"))?
            .clone();
        if profile.status == "unavailable" {
            bail!("launch profile is unavailable: {profile_id}");
        }
        let host = config
            .models
            .hosts
            .get(&profile.host)
            .with_context(|| format!("unknown model host: {}", profile.host))?;

        self.ensure_tunnel(host, config).await?;
        if self.is_expected_model(&profile).await.unwrap_or(false) {
            return Ok(profile);
        }

        budget.model_switch()?;
        audit.event(
            "model_switch_started",
            json!({"profile": profile.id, "host": profile.host}),
            &budget.usage,
        )?;
        self.start_remote(&profile, host, config).await?;

        let deadline = Instant::now()
            + Duration::from_secs(config.runtime.loop_limits.model_start_timeout_sec);
        loop {
            self.ensure_tunnel(host, config).await?;
            if self.is_expected_model(&profile).await.unwrap_or(false) {
                audit.event(
                    "model_ready",
                    json!({"profile": profile.id, "api_base": profile.api_base}),
                    &budget.usage,
                )?;
                return Ok(profile);
            }
            if Instant::now() >= deadline {
                audit.event(
                    "model_start_failed",
                    json!({"profile": profile.id, "reason": "timeout"}),
                    &budget.usage,
                )?;
                bail!("model did not become ready before timeout: {}", profile.id);
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    async fn is_expected_model(&self, profile: &LaunchProfile) -> Result<bool> {
        let expected = profile.model_name()?;
        let url = format!("{}/v1/models", profile.api_base.trim_end_matches('/'));
        let response = self.client.get(url).send().await?.error_for_status()?;
        let payload: ModelsResponse = response.json().await?;
        Ok(payload
            .data
            .iter()
            .chain(payload.models.iter())
            .flat_map(ModelEntry::names)
            .any(|name| model_name_matches(name, expected)))
    }

    async fn ensure_tunnel(&self, host: &HostConfig, config: &ProjectConfig) -> Result<()> {
        let Some(tunnel) = &host.tunnel else {
            return Ok(());
        };
        if tunnel_is_open(tunnel).await {
            return Ok(());
        }
        if host.ssh_target.is_empty() {
            bail!("SSH tunnel requested for a host without ssh_target");
        }
        let forward = format!(
            "{}:{}:{}:{}",
            tunnel.local_host, tunnel.local_port, tunnel.remote_host, tunnel.remote_port
        );
        let status = Command::new(&config.runtime.ssh.binary)
            .args([
                "-fN",
                "-i",
                &config.runtime.ssh.identity_file,
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "ServerAliveInterval=30",
                "-o",
                "ServerAliveCountMax=3",
                "-L",
                &forward,
                &host.ssh_target,
            ])
            .status()
            .await
            .context("failed to start SSH tunnel")?;
        if !status.success() {
            bail!("SSH tunnel command failed with status {status}");
        }
        for _ in 0..20 {
            if tunnel_is_open(tunnel).await {
                return Ok(());
            }
            sleep(Duration::from_millis(100)).await;
        }
        bail!("SSH tunnel did not become ready")
    }

    async fn start_remote(
        &self,
        profile: &LaunchProfile,
        host: &HostConfig,
        config: &ProjectConfig,
    ) -> Result<()> {
        if host.ssh_target.is_empty() {
            bail!("local launch profiles are not implemented yet");
        }
        validate_remote_path(&profile.launch_script)?;
        validate_identifier(&profile.id)?;
        let remote = format!(
            "state_dir=\"$HOME/.local/state/aiconductor\"; mkdir -p \"$state_dir\" && setsid -f {} >\"$state_dir/{}.log\" 2>&1 </dev/null",
            profile.launch_script, profile.id
        );
        let output = Command::new(&config.runtime.ssh.binary)
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                &format!("ConnectTimeout={}", config.runtime.ssh.connect_timeout_sec),
                "-i",
                &config.runtime.ssh.identity_file,
                &host.ssh_target,
                &remote,
            ])
            .output()
            .await
            .context("failed to invoke remote model launcher")?;
        if !output.status.success() {
            bail!(
                "remote model launcher failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

fn model_name_matches(observed: &str, expected: &str) -> bool {
    observed == expected
        || Path::new(observed)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == expected)
}

async fn tunnel_is_open(tunnel: &TunnelConfig) -> bool {
    let address = format!("{}:{}", tunnel.local_host, tunnel.local_port);
    let Ok(address) = address.parse::<SocketAddr>() else {
        return false;
    };
    timeout(Duration::from_millis(300), TcpStream::connect(address))
        .await
        .is_ok_and(|result| result.is_ok())
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("unsafe profile identifier: {value}");
    }
    Ok(())
}

fn validate_remote_path(value: &str) -> Result<()> {
    if !Path::new(value).is_absolute()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
    {
        bail!("unsafe remote launch path: {value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_shell_metacharacters_in_remote_paths() {
        assert!(validate_remote_path("/home/user/bin/start-model").is_ok());
        assert!(validate_remote_path("/home/user/bin/start;bad").is_err());
    }

    #[test]
    fn accepts_model_ids_returned_as_absolute_paths() {
        assert!(model_name_matches(
            "/srv/models/example-model.v1.gguf",
            "example-model.v1.gguf"
        ));
        assert!(!model_name_matches(
            "/srv/models/other.gguf",
            "example-model.v1.gguf"
        ));
    }

    #[test]
    fn accepts_dotted_profile_ids_without_allowing_path_or_shell_syntax() {
        assert!(validate_identifier("model.v1-quantized").is_ok());
        assert!(validate_identifier("llm-jp_4-8b").is_ok());
        assert!(validate_identifier("../profile").is_err());
        assert!(validate_identifier("profile;bad").is_err());
        assert!(validate_identifier("profile name").is_err());
    }
}
