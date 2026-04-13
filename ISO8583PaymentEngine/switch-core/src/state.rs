use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::routing::BinTrie;
use dashmap::DashMap;
use std::sync::atomic::AtomicBool;
use crossbeam::channel::Sender;
use crate::context::TransactionContext;

/// A STIP rule representing the action to take when the upstream bank is unreachable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StipRule {
    pub action: String,
}

/// A CRDB card record containing live memory cache of the card's financials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardRecord {
    pub balance: f64,
    pub is_blocked: bool,
    pub pvv: String,
}

/// The STIP Engine holds the live map of PANs -> STIP Rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StipEngine {
    pub rules: HashMap<String, StipRule>,
}

/// The CRDB Engine holds the live map of PANs -> Card Records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdbEngine {
    pub cards: HashMap<String, CardRecord>,
}

/// Global Zero-Downtime Hot-Reloadable Memory State for the Switch Core.
pub struct GlobalState {
    pub stip: ArcSwap<StipEngine>,
    pub crdb: ArcSwap<CrdbEngine>,
    pub router: ArcSwap<BinTrie>,
    pub node_health: DashMap<String, AtomicBool>,
    pub egress_channels: DashMap<String, Sender<TransactionContext>>,
}

impl GlobalState {
    pub fn new() -> Self {
        Self {
            stip: ArcSwap::from_pointee(StipEngine { rules: HashMap::new() }),
            crdb: ArcSwap::from_pointee(CrdbEngine { cards: HashMap::new() }),
            router: ArcSwap::from_pointee(BinTrie::new()),
            node_health: DashMap::new(),
            egress_channels: DashMap::new(),
        }
    }
}
