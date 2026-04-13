use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RouteDestination {
    InternalCrdb,
    ExternalNode(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Route {
    pub destination: RouteDestination,
    pub failover_node: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinTrieNode {
    pub route: Option<Route>,
    pub children: HashMap<u8, Box<BinTrieNode>>,
}

impl BinTrieNode {
    pub fn new() -> Self {
        Self {
            route: None,
            children: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinTrie {
    pub root: BinTrieNode,
}

impl BinTrie {
    pub fn new() -> Self {
        Self {
            root: BinTrieNode::new(),
        }
    }

    pub fn insert(&mut self, bin: &[u8], route: Route) {
        let mut curr = &mut self.root;
        for &b in bin {
            curr = curr.children.entry(b).or_insert_with(|| Box::new(BinTrieNode::new()));
        }
        curr.route = Some(route);
    }

    pub fn find(&self, pan: &[u8]) -> Option<&Route> {
        let mut curr = &self.root;
        let mut best_match = curr.route.as_ref();
        
        for &b in pan {
            if let Some(child) = curr.children.get(&b) {
                curr = child.as_ref();
                if curr.route.is_some() {
                    best_match = curr.route.as_ref();
                }
            } else {
                break;
            }
        }
        
        best_match
    }
}

impl Default for BinTrie {
    fn default() -> Self {
        Self::new()
    }
}
