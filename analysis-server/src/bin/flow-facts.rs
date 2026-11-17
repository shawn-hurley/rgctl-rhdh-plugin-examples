//! Evaluation-only export of native RGCTL facts. No independent source parsing.
use anyhow::{Context, Result, ensure};
use rgctl_analysis::{ProgramDependenceGraph, build_cfg_for_function};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf};

fn main() -> Result<()> {
    let roots: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(!roots.is_empty(), "provide fixture roots");
    for arg in roots {
        let root = PathBuf::from(arg).canonicalize()?;
        let graph: Value = serde_json::from_slice(&std::fs::read(root.join("graph.json"))?)?;
        ensure!(
            graph["provenance"]["rgctlSourceSha256"] == env!("RGCTL_SOURCE_SHA256"),
            "native probe and flow-facts builds differ; rerun probe"
        );
        let case: Value = serde_json::from_slice(&std::fs::read(root.join("case.json"))?)?;
        let language = case["language"].as_str().context("language")?;
        ensure!(
            ["java", "javascript", "typescript", "python", "go"].contains(&language),
            "unsupported language: {language}"
        );
        let mut sources = BTreeMap::new();
        for (relative, digest) in case["sourceDigests"]["baseline"]
            .as_object()
            .context("source digests")?
        {
            let file = root.join("baseline").join(relative).canonicalize()?;
            let source = std::fs::read_to_string(&file)?;
            ensure!(
                hex::encode(Sha256::digest(source.as_bytes()))
                    == digest.as_str().context("digest")?,
                "source changed: {relative}"
            );
            sources.insert(file, source);
        }
        let nodes = graph["baselineNodes"].as_array().context("nodes")?;
        let mut functions = vec![];
        for node in nodes.iter().filter(|n| n["node_type"] == "Function") {
            let name = node["name"].as_str().context("function name")?;
            let path = PathBuf::from(node["file_path"].as_str().context("function file")?)
                .canonicalize()?;
            let source = sources.get(&path).context("unhashed source")?;
            let duplicates = nodes
                .iter()
                .filter(|n| {
                    n["node_type"] == "Function"
                        && n["file_path"] == node["file_path"]
                        && n["name"] == name
                })
                .count();
            if duplicates != 1 {
                functions.push(json!({"id":node["id"],"status":"unavailable","reason":"name-based native CFG API is ambiguous in this file"}));
                continue;
            }
            match build_cfg_for_function(language, source, name) {
                Ok(cfg) => {
                    let pdg = ProgramDependenceGraph::build(&cfg, source.as_bytes());
                    match pdg {
                        Ok(pdg) => functions.push(json!({"id":node["id"],"status":"available","cfg":cfg,"localTypes":cfg.local_types,"pdg":pdg})),
                        Err(e) => functions.push(json!({"id":node["id"],"status":"unavailable","reason":format!("PDG: {e}"),"cfg":cfg,"localTypes":cfg.local_types})),
                    }
                }
                Err(e) => functions.push(
                    json!({"id":node["id"],"status":"unavailable","reason":format!("CFG: {e}")}),
                ),
            }
        }
        let output = json!({"schemaVersion":1,"origin":"rgctl-crate-apis","provenance":graph["provenance"],"sourceDigests":case["sourceDigests"]["baseline"],"functions":functions});
        std::fs::write(
            root.join("flow-facts.json"),
            serde_json::to_vec_pretty(&output)?,
        )?;
        println!(
            "{}: {} native function records",
            root.display(),
            functions.len()
        );
    }
    Ok(())
}
