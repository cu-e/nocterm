//! The arena of a view as a tree.

use nocterm_plugin_api::{Node, View, ViewError};

/// A validated view with the children of each node.
pub(crate) struct Tree {
    nodes: Vec<Node>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
}

impl Tree {
    /// Sanitizes and validates a view; a view the host would refuse is an error.
    pub(crate) fn new(view: View) -> Result<Self, ViewError> {
        let view = view.into_validated()?;
        let mut children = vec![Vec::new(); view.nodes.len()];
        let mut roots = Vec::new();
        for (index, node) in view.nodes.iter().enumerate() {
            match node.parent {
                Some(parent) => children[usize::from(parent)].push(index),
                None => roots.push(index),
            }
        }
        Ok(Self {
            nodes: view.nodes,
            children,
            roots,
        })
    }

    pub(crate) fn roots(&self) -> &[usize] {
        &self.roots
    }

    pub(crate) fn node(&self, index: usize) -> &Node {
        &self.nodes[index]
    }

    pub(crate) fn children(&self, index: usize) -> &[usize] {
        &self.children[index]
    }
}
