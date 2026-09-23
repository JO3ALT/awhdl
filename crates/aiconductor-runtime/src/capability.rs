//! One parameterized capability model for tool, filesystem, sandbox, network and
//! model access. Everything not granted is denied.
use crate::effect::{EffectClass, sha256_hex};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A grant: `action` on a `kind:pattern` resource, under constraints, with the
/// effect class that drives retry, approval and audit policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    /// Route names holding this capability. Agents get handles, never credentials.
    #[serde(default)]
    pub subjects: Vec<String>,
    pub action: String,
    pub resource: String,
    /// Every request parameter must be listed here and match its glob.
    #[serde(default)]
    pub constraints: BTreeMap<String, String>,
    pub effect_class: EffectClass,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRequest {
    pub action: String,
    pub resource: String,
    pub params: BTreeMap<String, String>,
}

impl CapabilityRequest {
    pub fn new(action: &str, resource: impl Into<String>) -> Self {
        Self {
            action: action.to_owned(),
            resource: resource.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn param(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    pub capability_id: String,
    /// `id#fingerprint`; changes whenever the grant's permission scope changes.
    pub handle: String,
    pub effect_class: EffectClass,
}

impl Capability {
    /// Hash of the permission scope. Holders and revocation are not scope.
    pub fn fingerprint(&self) -> String {
        let scope = serde_json::json!({
            "id": self.id,
            "action": self.action,
            "resource": self.resource,
            "constraints": self.constraints,
            "effect_class": self.effect_class,
        });
        sha256_hex(&serde_json::to_vec(&scope).expect("serializable"))
    }

    pub fn handle(&self) -> String {
        format!("{}#{}", self.id, &self.fingerprint()[..16])
    }

    fn permits(&self, request: &CapabilityRequest) -> bool {
        !self.revoked
            && self.action == request.action
            && resource_matches(&self.resource, &request.resource)
            && request.params.iter().all(|(key, value)| {
                self.constraints
                    .get(key)
                    .is_some_and(|pattern| glob_segment(pattern, value))
            })
            && self
                .constraints
                .keys()
                .all(|key| request.params.contains_key(key))
    }

    /// Derive a capability that can do no more than this one: same action, a
    /// resource inside this resource, the same or tighter constraints, and an
    /// effect class no higher than this one.
    pub fn narrow(
        &self,
        id: &str,
        resource: &str,
        constraints: BTreeMap<String, String>,
        effect_class: EffectClass,
    ) -> Result<Self> {
        ensure!(!self.revoked, "cannot narrow a revoked capability");
        ensure!(
            resource_within(resource, &self.resource),
            "narrowed resource is not within {}",
            self.resource
        );
        ensure!(
            effect_class.rank() <= self.effect_class.rank(),
            "narrowing cannot raise the effect class"
        );
        ensure!(
            constraints.keys().eq(self.constraints.keys()),
            "narrowing must keep exactly the parent's constraint keys"
        );
        for (key, pattern) in &constraints {
            ensure!(
                !has_wildcard(pattern) && glob_segment(&self.constraints[key], pattern)
                    || pattern == &self.constraints[key],
                "constraint {key} is not narrower"
            );
        }
        let narrowed = Self {
            id: id.to_owned(),
            subjects: self.subjects.clone(),
            action: self.action.clone(),
            resource: resource.to_owned(),
            constraints,
            effect_class,
            revoked: false,
        };
        narrowed.validate()?;
        Ok(narrowed)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty() && !self.id.contains('#'),
            "invalid capability id"
        );
        ensure!(
            !self.action.is_empty(),
            "capability {} has no action",
            self.id
        );
        let (kind, pattern) = self
            .resource
            .split_once(':')
            .with_context(|| format!("capability {} resource needs kind:pattern", self.id))?;
        ensure!(
            !kind.is_empty() && !pattern.is_empty(),
            "capability {} has an empty resource",
            self.id
        );
        if kind == "file" {
            ensure!(
                !pattern.starts_with('/')
                    && !pattern.contains('\\')
                    && pattern
                        .split('/')
                        .all(|s| !s.is_empty() && s != "." && s != ".."),
                "capability {} file pattern must be project-relative and normalized",
                self.id
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPolicy {
    pub schema_version: u32,
    #[serde(default)]
    pub grants: Vec<Capability>,
}

impl CapabilityPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported capability schema version"
        );
        let mut ids = std::collections::BTreeSet::new();
        for grant in &self.grants {
            grant.validate()?;
            ensure!(ids.insert(&grant.id), "duplicate capability {}", grant.id);
        }
        Ok(())
    }

    /// The single policy question. When several grants match, the most severe
    /// effect class wins, so an overlapping broad grant cannot understate risk.
    pub fn authorize(&self, subject: &str, request: &CapabilityRequest) -> Result<Authorization> {
        let grant = self
            .grants
            .iter()
            .filter(|grant| grant.subjects.iter().any(|s| s == subject) && grant.permits(request))
            .max_by_key(|grant| grant.effect_class.rank())
            .with_context(|| {
                format!(
                    "capability denied: {subject} may not {} {}",
                    request.action, request.resource
                )
            })?;
        Ok(Authorization {
            capability_id: grant.id.clone(),
            handle: grant.handle(),
            effect_class: grant.effect_class,
        })
    }

    pub fn revoke(&mut self, id: &str) -> Result<()> {
        let grant = self
            .grants
            .iter_mut()
            .find(|grant| grant.id == id)
            .with_context(|| format!("unknown capability {id}"))?;
        grant.revoked = true;
        Ok(())
    }
}

/// `file:<project-relative path>` for an existing path. Symlinks are resolved,
/// so a link cannot smuggle an outside path into the project namespace.
pub fn file_resource(root: &Path, value: &str) -> Result<String> {
    let root = root.canonicalize()?;
    let candidate = Path::new(value);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let path = joined
        .canonicalize()
        .with_context(|| format!("path does not exist: {value}"))?;
    let relative = path
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("capability denied: path is outside the project: {value}"))?;
    let parts = relative
        .components()
        .map(|part| part.as_os_str().to_str().context("non-UTF-8 path"))
        .collect::<Result<Vec<_>>>()?;
    if parts.is_empty() {
        bail!("capability denied: project root itself is not a file resource");
    }
    Ok(format!("file:{}", parts.join("/")))
}

fn has_wildcard(pattern: &str) -> bool {
    pattern.contains('*')
}

fn resource_matches(pattern: &str, resource: &str) -> bool {
    match (pattern.split_once(':'), resource.split_once(':')) {
        (Some((kind, pattern)), Some((actual_kind, value))) => {
            kind == actual_kind && glob_path(pattern, value)
        }
        _ => false,
    }
}

/// Conservative containment: equal, or the parent ends in `/**` and the child
/// lies under its literal prefix, or the child is literal and the parent matches it.
fn resource_within(child: &str, parent: &str) -> bool {
    if child == parent {
        return true;
    }
    let (Some((child_kind, child_value)), Some((parent_kind, parent_value))) =
        (child.split_once(':'), parent.split_once(':'))
    else {
        return false;
    };
    if child_kind != parent_kind {
        return false;
    }
    if !has_wildcard(child_value) {
        return glob_path(parent_value, child_value);
    }
    if parent_value == "**" {
        return true;
    }
    parent_value
        .strip_suffix("/**")
        .filter(|prefix| !has_wildcard(prefix))
        .is_some_and(|prefix| child_value.starts_with(&format!("{prefix}/")))
}

/// `**` matches any number of segments; `*` matches within one segment.
fn glob_path(pattern: &str, value: &str) -> bool {
    fn go(pattern: &[&str], value: &[&str]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((&"**", rest)) => (0..=value.len()).any(|skip| go(rest, &value[skip..])),
            Some((segment, rest)) => value
                .split_first()
                .is_some_and(|(head, tail)| glob_segment(segment, head) && go(rest, tail)),
        }
    }
    let pattern = pattern.split('/').collect::<Vec<_>>();
    let value = value.split('/').collect::<Vec<_>>();
    go(&pattern, &value)
}

fn glob_segment(pattern: &str, value: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = value.strip_prefix(first) else {
        return false;
    };
    let parts = parts.collect::<Vec<_>>();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(id: &str, action: &str, resource: &str, class: EffectClass) -> Capability {
        Capability {
            id: id.to_owned(),
            subjects: vec!["agent".to_owned()],
            action: action.to_owned(),
            resource: resource.to_owned(),
            constraints: BTreeMap::new(),
            effect_class: class,
            revoked: false,
        }
    }

    fn policy(grants: Vec<Capability>) -> CapabilityPolicy {
        let policy = CapabilityPolicy {
            schema_version: 1,
            grants,
        };
        policy.validate().unwrap();
        policy
    }

    #[test]
    fn workspace_paths_are_scoped_and_outside_paths_are_denied() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("examples/a")).unwrap();
        std::fs::create_dir_all(root.path().join(".runtime/secrets")).unwrap();
        std::fs::write(root.path().join("examples/a/x.pl"), "").unwrap();
        std::fs::write(root.path().join(".runtime/secrets/key"), "").unwrap();
        std::fs::write(outside.path().join("y.pl"), "").unwrap();
        let policy = policy(vec![grant(
            "examples_read",
            "file.read",
            "file:examples/**",
            EffectClass::Read,
        )]);
        let read = |value: &str| {
            file_resource(root.path(), value).and_then(|resource| {
                policy.authorize("agent", &CapabilityRequest::new("file.read", resource))
            })
        };
        assert!(read("examples/a/x.pl").is_ok());
        assert!(read(&root.path().join("examples/a/x.pl").to_string_lossy()).is_ok());
        assert!(read(".runtime/secrets/key").is_err());
        assert!(read("examples/../.runtime/secrets/key").is_err());
        assert!(read(&outside.path().join("y.pl").to_string_lossy()).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), root.path().join("examples/link")).unwrap();
            assert!(read("examples/link/y.pl").is_err());
        }
        // A different subject or action does not inherit the grant.
        let resource = file_resource(root.path(), "examples/a/x.pl").unwrap();
        assert!(
            policy
                .authorize(
                    "other",
                    &CapabilityRequest::new("file.read", resource.clone())
                )
                .is_err()
        );
        assert!(
            policy
                .authorize("agent", &CapabilityRequest::new("file.write", resource))
                .is_err()
        );
    }

    #[test]
    fn push_and_connect_are_constrained_by_parameters() {
        let mut push = grant(
            "vai_push",
            "github.push",
            "repo:JO3ALT/vai",
            EffectClass::ExternalWrite,
        );
        push.constraints
            .insert("branch".to_owned(), "dev".to_owned());
        let mut connect = grant(
            "openai",
            "network.connect",
            "host:api.openai.com",
            EffectClass::ExternalWrite,
        );
        connect
            .constraints
            .insert("port".to_owned(), "443".to_owned());
        let policy = policy(vec![push, connect]);
        let push = |repo: &str, branch: &str| {
            policy.authorize(
                "agent",
                &CapabilityRequest::new("github.push", format!("repo:{repo}"))
                    .param("branch", branch),
            )
        };
        assert!(push("JO3ALT/vai", "dev").is_ok());
        assert!(push("JO3ALT/vai", "main").is_err());
        assert!(push("other/repo", "dev").is_err());
        // Unlisted parameters are denied; listed constraints are mandatory.
        assert!(
            policy
                .authorize(
                    "agent",
                    &CapabilityRequest::new("github.push", "repo:JO3ALT/vai")
                        .param("branch", "dev")
                        .param("force", "true"),
                )
                .is_err()
        );
        assert!(
            policy
                .authorize(
                    "agent",
                    &CapabilityRequest::new("github.push", "repo:JO3ALT/vai")
                )
                .is_err()
        );
        let connect = |host: &str, port: &str| {
            policy.authorize(
                "agent",
                &CapabilityRequest::new("network.connect", format!("host:{host}"))
                    .param("port", port),
            )
        };
        assert!(connect("api.openai.com", "443").is_ok());
        assert!(connect("api.openai.com", "80").is_err());
        assert!(connect("evil.example", "443").is_err());
    }

    #[test]
    fn narrowing_only_reduces_scope_and_revocation_denies() {
        let mut broad = grant("files", "file.read", "file:**", EffectClass::Read);
        broad.constraints.insert("mode".to_owned(), "*".to_owned());
        let narrow = broad
            .narrow(
                "examples",
                "file:examples/**",
                BTreeMap::from([("mode".to_owned(), "text".to_owned())]),
                EffectClass::Read,
            )
            .unwrap();
        assert_ne!(narrow.handle(), broad.handle());
        let within_narrow = narrow
            .narrow(
                "one",
                "file:examples/a/**",
                narrow.constraints.clone(),
                EffectClass::Pure,
            )
            .unwrap();
        assert!(
            within_narrow
                .narrow(
                    "up",
                    "file:**",
                    narrow.constraints.clone(),
                    EffectClass::Pure
                )
                .is_err()
        );
        assert!(
            narrow
                .narrow(
                    "sib",
                    "file:docs/**",
                    narrow.constraints.clone(),
                    EffectClass::Read
                )
                .is_err()
        );
        assert!(
            narrow
                .narrow(
                    "esc",
                    "file:examples/**",
                    narrow.constraints.clone(),
                    EffectClass::LocalWrite
                )
                .is_err()
        );
        assert!(
            narrow
                .narrow(
                    "loose",
                    "file:examples/**",
                    BTreeMap::from([("mode".to_owned(), "*".to_owned())]),
                    EffectClass::Read
                )
                .is_err()
        );
        assert!(
            narrow
                .narrow(
                    "drop",
                    "file:examples/**",
                    BTreeMap::new(),
                    EffectClass::Read
                )
                .is_err()
        );
        let request =
            CapabilityRequest::new("file.read", "file:examples/x.pl").param("mode", "text");
        let mut policy = policy(vec![narrow]);
        assert!(policy.authorize("agent", &request).is_ok());
        policy.revoke("examples").unwrap();
        assert!(policy.authorize("agent", &request).is_err());
        let revoked = &policy.grants[0];
        assert!(
            revoked
                .narrow(
                    "again",
                    "file:examples/a/**",
                    revoked.constraints.clone(),
                    EffectClass::Read
                )
                .is_err()
        );
    }

    #[test]
    fn overlapping_grants_report_the_most_severe_class() {
        let policy = policy(vec![
            grant("kdb_read", "mcp.call", "mcp:kdb/get_*", EffectClass::Read),
            grant("kdb_all", "mcp.call", "mcp:kdb/*", EffectClass::LocalWrite),
        ]);
        let auth = policy
            .authorize(
                "agent",
                &CapabilityRequest::new("mcp.call", "mcp:kdb/get_state"),
            )
            .unwrap();
        assert_eq!(auth.effect_class, EffectClass::LocalWrite);
        assert_eq!(auth.capability_id, "kdb_all");
        assert!(glob_path("a/**/c", "a/c") && glob_path("a/**/c", "a/b/b/c"));
        assert!(!glob_path("a/*", "a/b/c") && glob_segment("get_*_x", "get_a_x"));
        let bad = CapabilityPolicy {
            schema_version: 1,
            grants: vec![grant("abs", "file.read", "file:/etc/**", EffectClass::Read)],
        };
        assert!(bad.validate().is_err());
        let bad = CapabilityPolicy {
            schema_version: 1,
            grants: vec![grant(
                "dots",
                "file.read",
                "file:examples/../x",
                EffectClass::Read,
            )],
        };
        assert!(bad.validate().is_err());
    }
}
