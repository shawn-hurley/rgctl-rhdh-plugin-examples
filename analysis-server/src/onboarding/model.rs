use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub data_dir: PathBuf,
    pub allowed_repository_hosts: Vec<String>,
    #[serde(default)]
    pub allowed_source_roots: Vec<PathBuf>,
    pub maven_repository: Option<PathBuf>,
    #[serde(default = "queue_capacity")]
    pub queue_capacity: usize,
}
fn queue_capacity() -> usize {
    8
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub repository_url: String,
    #[serde(default = "revision")]
    pub revision: String,
    #[serde(default)]
    pub sub_path: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_maven_dependencies: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn revision() -> String {
    "HEAD".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Onboard {
    pub entity_ref: String,
    pub source: Source,
    #[serde(default)]
    pub provides_apis: Vec<String>,
    #[serde(default)]
    pub consumes_apis: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub request: Onboard,
    pub state: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub graph_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub node_id: String,
    pub qualified_name: String,
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub direction: String,
    pub framework: String,
    pub methods: Vec<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub symbol: Symbol,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub application_key: String,
    pub entity_ref: String,
    pub graph_version: String,
    pub source: Source,
    pub commit: String,
    pub source_sha256: String,
    pub snapshot_sha256: String,
    pub rgctl_revision: String,
    pub rgctl_source_sha256: String,
    pub files_indexed: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub provides_apis: Vec<String>,
    pub consumes_apis: Vec<String>,
    pub operations: Vec<Operation>,
    pub diagnostics: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_coordinate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<LibrarySource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_traces: Vec<DependencyTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_uses: Vec<ResourceUse>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySource {
    pub coordinate: String,
    pub prefix: String,
    pub direct: bool,
    pub status: String,
    pub source_sha256: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodePath {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Value>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyTrace {
    pub coordinate: String,
    pub paths: Vec<CodePath>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUse {
    pub resource_type: String,
    pub uri: Option<String>,
    pub operation: String,
    pub symbol: Symbol,
    pub application_paths: Vec<CodePath>,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn entity_ref(value: &str, kind: &str) -> bool {
    let Some(rest) = value.strip_prefix(&format!("{kind}:")) else {
        return false;
    };
    let Some((namespace, name)) = rest.split_once('/') else {
        return false;
    };
    [namespace, name].iter().all(|s| {
        !s.is_empty()
            && s.len() <= 63
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    })
}
