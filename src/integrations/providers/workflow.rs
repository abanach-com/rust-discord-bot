use crate::integrations::{CapabilityStatus, ProviderAdapter, ProviderCapability};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub node_type: String, // "trigger" | "condition" | "action" | "delay"
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDag {
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

impl WorkflowDag {
    /// Validates that the workflow contains no cycles using Kahn's topological sort.
    pub fn validate_no_cycles(&self) -> Result<(), String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for node in &self.nodes {
            in_degree.insert(&node.id, 0);
            adj.insert(&node.id, Vec::new());
        }

        for edge in &self.edges {
            if !in_degree.contains_key(edge.from.as_str())
                || !in_degree.contains_key(edge.to.as_str())
            {
                return Err(format!(
                    "Edge references non-existent node: {} -> {}",
                    edge.from, edge.to
                ));
            }
            adj.get_mut(edge.from.as_str())
                .unwrap()
                .push(edge.to.as_str());
            *in_degree.get_mut(edge.to.as_str()).unwrap() += 1;
        }

        let mut queue: Vec<&str> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut visited = 0;
        while let Some(u) = queue.pop() {
            visited += 1;
            if let Some(neighbors) = adj.get(u) {
                for &v in neighbors {
                    let deg = in_degree.get_mut(v).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(v);
                    }
                }
            }
        }

        if visited != self.nodes.len() {
            return Err("Cycle detected in workflow graph".to_string());
        }

        Ok(())
    }
}

pub struct WorkflowBuilderAdapter;

impl ProviderAdapter for WorkflowBuilderAdapter {
    fn provider_name(&self) -> &'static str {
        "workflow"
    }

    fn capabilities(&self) -> Vec<ProviderCapability> {
        vec![
            ProviderCapability {
                name: "dag_execution".to_string(),
                description: "Visual DAG node evaluation with step isolation".to_string(),
                status: CapabilityStatus::Supported,
                read_only: false,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "cycle_detection".to_string(),
                description: "Topological cycle and fan-out validation".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
            ProviderCapability {
                name: "dry_run".to_string(),
                description: "Simulate execution on sample inputs without mutations".to_string(),
                status: CapabilityStatus::Supported,
                read_only: true,
                requires_write_scope: false,
            },
        ]
    }
}
