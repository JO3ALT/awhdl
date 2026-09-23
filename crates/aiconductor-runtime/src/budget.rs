use crate::config::LoopLimits;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub iterations: u32,
    pub model_calls: u32,
    pub mcp_calls: u32,
    pub model_switches: u32,
    pub network_requests: u32,
    pub downloaded_bytes: u64,
}

pub struct Budget {
    limits: LoopLimits,
    started: Instant,
    pub usage: BudgetUsage,
}

impl Budget {
    pub fn new(limits: LoopLimits) -> Self {
        Self {
            limits,
            started: Instant::now(),
            usage: BudgetUsage::default(),
        }
    }

    /// Apply a workflow's own limits; they can only lower the configured ones.
    pub fn tighten(
        &mut self,
        iterations: Option<u32>,
        mcp_calls: Option<u32>,
        model_calls: Option<u32>,
    ) {
        let lower = |limit: &mut u32, value: Option<u32>| {
            if let Some(value) = value {
                *limit = (*limit).min(value);
            }
        };
        lower(&mut self.limits.max_iterations, iterations);
        lower(&mut self.limits.max_mcp_calls, mcp_calls);
        lower(&mut self.limits.max_model_calls, model_calls);
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn check_time(&self) -> Result<()> {
        if self.elapsed().as_secs() > self.limits.max_wall_time_sec {
            bail!("wall-time budget exhausted");
        }
        Ok(())
    }

    pub fn iteration(&mut self) -> Result<()> {
        self.check_time()?;
        self.usage.iterations += 1;
        if self.usage.iterations > self.limits.max_iterations {
            bail!("iteration budget exhausted");
        }
        Ok(())
    }

    pub fn model_call(&mut self) -> Result<()> {
        self.check_time()?;
        self.usage.model_calls += 1;
        if self.usage.model_calls > self.limits.max_model_calls {
            bail!("model-call budget exhausted");
        }
        Ok(())
    }

    pub fn mcp_call(&mut self) -> Result<()> {
        self.check_time()?;
        self.usage.mcp_calls += 1;
        if self.usage.mcp_calls > self.limits.max_mcp_calls {
            bail!("MCP-call budget exhausted");
        }
        Ok(())
    }

    pub fn model_switch(&mut self) -> Result<()> {
        self.check_time()?;
        self.usage.model_switches += 1;
        if self.usage.model_switches > self.limits.max_model_switches {
            bail!("model-switch budget exhausted");
        }
        Ok(())
    }

    pub fn network_request(&mut self) -> Result<()> {
        self.usage.network_requests += 1;
        if self.usage.network_requests > self.limits.max_network_requests {
            bail!("network-request budget exhausted");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> LoopLimits {
        LoopLimits {
            max_iterations: 1,
            max_wall_time_sec: 60,
            max_model_calls: 1,
            max_mcp_calls: 1,
            max_model_switches: 1,
            max_network_requests: 1,
            max_download_bytes: 1,
            model_start_timeout_sec: 1,
            device_timeout_sec: 1,
            planner_retries: 1,
        }
    }

    #[test]
    fn rejects_iteration_overrun() {
        let mut budget = Budget::new(limits());
        budget.iteration().unwrap();
        assert!(budget.iteration().is_err());
    }
}
