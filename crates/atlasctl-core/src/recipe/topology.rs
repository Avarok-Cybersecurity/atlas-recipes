// SPDX-License-Identifier: AGPL-3.0-only

//! Node-count normalization.

use super::raw::RawRecipe;

/// How many nodes a recipe runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Topology {
    /// Fewest nodes the recipe can run on.
    pub min_nodes: u32,
    /// Most nodes it can run on, when bounded.
    pub max_nodes: Option<u32>,
}

impl Topology {
    /// Normalize `min_nodes` / `max_nodes` / `solo_only` / `cluster_only`.
    ///
    /// `solo_only` and `cluster_only` are shorthands the reference implementation
    /// expands into node bounds; we do the same so both spellings behave
    /// identically. When a recipe sets neither, it is single-node.
    pub fn from_raw(raw: &RawRecipe) -> Self {
        let mut min = raw.min_nodes.unwrap_or(1).max(1);
        let mut max = raw.max_nodes;

        if raw.solo_only.unwrap_or(false) {
            max = Some(1);
        }
        if raw.cluster_only.unwrap_or(false) {
            min = min.max(2);
        }
        // A max below the min is a contradiction; trust the explicit floor.
        if let Some(m) = max
            && m < min
        {
            max = Some(min);
        }
        Self {
            min_nodes: min,
            max_nodes: max,
        }
    }

    /// Whether this recipe needs more than one node.
    pub fn is_multi_node(&self) -> bool {
        self.min_nodes > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(yaml: &str) -> RawRecipe {
        serde_yaml_ng::from_str(yaml).expect("fixture parses")
    }

    #[test]
    fn absent_bounds_mean_single_node() {
        let t = Topology::from_raw(&raw("model: m\n"));
        assert_eq!(
            t,
            Topology {
                min_nodes: 1,
                max_nodes: None
            }
        );
        assert!(!t.is_multi_node());
    }

    #[test]
    fn ep2_recipes_are_multi_node() {
        // The shape all three shipping EP=2 recipes use.
        let t = Topology::from_raw(&raw("model: m\nmin_nodes: 2\nmax_nodes: 2\n"));
        assert_eq!(
            t,
            Topology {
                min_nodes: 2,
                max_nodes: Some(2)
            }
        );
        assert!(t.is_multi_node());
    }

    #[test]
    fn solo_only_caps_at_one_node() {
        let t = Topology::from_raw(&raw("model: m\nsolo_only: true\n"));
        assert_eq!(t.max_nodes, Some(1));
        assert!(!t.is_multi_node());
    }

    #[test]
    fn cluster_only_raises_the_floor() {
        let t = Topology::from_raw(&raw("model: m\ncluster_only: true\n"));
        assert!(t.is_multi_node());
        assert_eq!(t.min_nodes, 2);
    }

    #[test]
    fn a_max_below_the_min_is_lifted_to_the_min() {
        let t = Topology::from_raw(&raw("model: m\nmin_nodes: 4\nmax_nodes: 2\n"));
        assert_eq!(
            t,
            Topology {
                min_nodes: 4,
                max_nodes: Some(4)
            }
        );
    }

    #[test]
    fn zero_nodes_is_treated_as_one() {
        assert_eq!(
            Topology::from_raw(&raw("model: m\nmin_nodes: 0\n")).min_nodes,
            1
        );
    }
}
