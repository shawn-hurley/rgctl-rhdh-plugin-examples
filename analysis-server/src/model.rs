use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub applications: BTreeMap<String, Application>,
    pub data_dir: PathBuf,
    pub maven_repository: Option<PathBuf>,
    #[serde(default = "default_queue")]
    pub queue_capacity: usize,
}
fn default_queue() -> usize {
    8
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Application {
    pub path: PathBuf,
    pub revision: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Comparison {
    pub entity_ref: String,
    pub application_revision: String,
    pub dependency: Coordinate,
    pub target_version: String,
    pub strategy: Strategy,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    DirectUpgrade,
    TransitiveOverride,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Coordinate {
    pub group_id: String,
    pub artifact_id: String,
    #[serde(default = "jar", rename = "type")]
    pub kind: String,
    pub classifier: Option<String>,
}
fn jar() -> String {
    "jar".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub request: Comparison,
    pub state: String,
    pub stage: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub error: Option<Problem>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 200
        && !s.starts_with('.')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
impl Comparison {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            safe_token(&self.dependency.group_id)
                && safe_token(&self.dependency.artifact_id)
                && safe_token(&self.target_version),
            "Invalid dependency coordinates or target version"
        );
        anyhow::ensure!(
            self.dependency.kind == "jar" && self.dependency.classifier.is_none(),
            "This version supports unclassified Maven JAR dependencies only"
        );
        Ok(())
    }
}
