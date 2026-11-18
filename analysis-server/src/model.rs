use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub applications: BTreeMap<String, Application>,
    pub data_dir: PathBuf,
    pub maven_repository: Option<PathBuf>,
    pub go_proxy: Option<String>,
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
#[serde(untagged)]
pub enum Coordinate {
    Maven(MavenCoordinate),
    Go(GoCoordinate),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoCoordinate {
    pub module_path: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MavenCoordinate {
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
        match &self.dependency {
            Coordinate::Maven(d) => {
                anyhow::ensure!(
                    safe_token(&d.group_id)
                        && safe_token(&d.artifact_id)
                        && safe_token(&self.target_version),
                    "Invalid Maven coordinates or target version"
                );
                anyhow::ensure!(
                    d.kind == "jar" && d.classifier.is_none(),
                    "Only unclassified Maven JAR dependencies are supported"
                );
            }
            Coordinate::Go(d) => {
                anyhow::ensure!(valid_module_path(&d.module_path), "Invalid Go module path");
                anyhow::ensure!(
                    self.target_version.starts_with('v')
                        && self.target_version[1..].split('.').take(3).count() == 3
                        && safe_token(&self.target_version.replace('+', "-")),
                    "Invalid Go module version"
                );
            }
        }
        Ok(())
    }
}
pub fn valid_module_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('-')
        && path.len() < 500
        && path.split('/').all(|part| safe_token(part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn coordinates_preserve_maven_and_accept_go_without_cross_format_fields() {
        let request = |dependency| json!({"entityRef":"component:default/app","applicationRevision":"main","dependency":dependency,"targetVersion":"v1.0.1","strategy":"direct-upgrade"});
        let java: Comparison =
            serde_json::from_value(request(json!({"groupId":"example","artifactId":"library"})))
                .unwrap();
        assert!(java.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&java).unwrap()["dependency"]["type"],
            "jar"
        );
        for path in ["example.test/risk", "github.com/Example/library/v2"] {
            let go: Comparison =
                serde_json::from_value(request(json!({"modulePath":path}))).unwrap();
            assert!(go.validate().is_ok());
            assert_eq!(
                serde_json::to_value(go).unwrap()["dependency"],
                json!({"modulePath":path})
            );
        }
        for path in [
            "../risk",
            "/tmp/risk",
            "example.test//risk",
            "example.test/risk@latest",
            "-flag",
        ] {
            let go: Comparison =
                serde_json::from_value(request(json!({"modulePath":path}))).unwrap();
            assert!(go.validate().is_err(), "{path}");
        }
        assert!(
            serde_json::from_value::<Comparison>(request(
                json!({"modulePath":"example.test/risk","groupId":"x","artifactId":"y"})
            ))
            .is_err()
        );
    }
}
