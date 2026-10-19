//! Method × field matrix of one type and candidate state owners.
//!
//! Fields that the same methods change together are candidates for one owner.
//! Coordinators (methods reaching most of the state) are excluded from the
//! similarity, otherwise every field would join one cluster through them.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Two fields join a cluster when this share of their methods is common.
const SIMILARITY: f64 = 0.5;

#[derive(Serialize)]
pub(super) struct Matrix {
    pub(super) owner: String,
    pub(super) fields: Vec<String>,
    pub(super) methods: Vec<Row>,
    pub(super) coordinators: Vec<String>,
    pub(super) clusters: Vec<Cluster>,
    pub(super) cross_cluster: Vec<String>,
    pub(super) no_direct_fields: Vec<String>,
    pub(super) untouched_fields: Vec<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct Row {
    pub(super) name: String,
    pub(super) file: String,
    pub(super) lines: usize,
    pub(super) touches: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct Cluster {
    pub(super) fields: Vec<String>,
    pub(super) methods: Vec<String>,
}

pub(super) fn build(owner: &str, fields: &[String], mut methods: Vec<Row>) -> Matrix {
    let known: BTreeSet<_> = fields.iter().collect();
    for row in &mut methods {
        row.touches.retain(|field| known.contains(field));
    }
    let coordinator_limit = (fields.len() / 2).max(3);
    let (coordinators, focused): (Vec<_>, Vec<_>) = methods
        .iter()
        .partition(|row| row.touches.len() > coordinator_limit);
    let clusters = clusters(fields, &focused);
    let cluster_of: BTreeMap<&String, usize> = clusters
        .iter()
        .enumerate()
        .flat_map(|(index, cluster)| cluster.iter().map(move |field| (field, index)))
        .collect();
    let mut grouped: Vec<Cluster> = clusters
        .iter()
        .map(|fields| Cluster {
            fields: fields.clone(),
            methods: Vec::new(),
        })
        .collect();
    let mut cross_cluster = Vec::new();
    let mut no_direct_fields = Vec::new();
    for row in &focused {
        let owners: BTreeSet<_> = row.touches.iter().map(|field| cluster_of[field]).collect();
        match owners.len() {
            0 => no_direct_fields.push(row.name.clone()),
            1 => grouped[*owners.first().unwrap_or(&0)]
                .methods
                .push(row.name.clone()),
            _ => cross_cluster.push(row.name.clone()),
        }
    }
    let touched: BTreeSet<_> = methods.iter().flat_map(|row| &row.touches).collect();
    grouped.retain(|cluster| !cluster.methods.is_empty());
    grouped.sort_by_key(|cluster| std::cmp::Reverse(cluster.fields.len()));
    Matrix {
        owner: owner.to_owned(),
        fields: fields.to_vec(),
        coordinators: coordinators.iter().map(|row| row.name.clone()).collect(),
        clusters: grouped,
        cross_cluster,
        no_direct_fields,
        untouched_fields: fields
            .iter()
            .filter(|field| !touched.contains(field))
            .cloned()
            .collect(),
        methods,
    }
}

/// Single-linkage clustering of fields by the Jaccard similarity of the
/// focused methods touching them.
fn clusters(fields: &[String], focused: &[&Row]) -> Vec<Vec<String>> {
    let users: Vec<BTreeSet<&str>> = fields
        .iter()
        .map(|field| {
            focused
                .iter()
                .filter(|row| row.touches.contains(field))
                .map(|row| row.name.as_str())
                .collect()
        })
        .collect();
    let mut parent: Vec<usize> = (0..fields.len()).collect();
    for left in 0..fields.len() {
        for right in left + 1..fields.len() {
            if similarity(&users[left], &users[right]) >= SIMILARITY {
                let (a, b) = (root(&mut parent, left), root(&mut parent, right));
                parent[b] = a;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (index, field) in fields.iter().enumerate() {
        if !users[index].is_empty() {
            let group = root(&mut parent, index);
            groups.entry(group).or_default().push(field.clone());
        }
    }
    groups.into_values().collect()
}

fn similarity(left: &BTreeSet<&str>, right: &BTreeSet<&str>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / union as f64
}

fn root(parent: &mut [usize], mut index: usize) -> usize {
    while parent[index] != index {
        parent[index] = parent[parent[index]];
        index = parent[index];
    }
    index
}
