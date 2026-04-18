use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::routing::BinTrie;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use crossbeam::channel::Sender;
use crate::context::TransactionContext;

/// A STIP rule representing the action to take when the upstream bank is unreachable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StipRule {
    pub action: String,
    /// Maximum acceptable network risk score (0–100).
    /// Transactions whose `risk_score` exceeds this threshold are instantly
    /// declined with RC 59 (Suspected Fraud) before any online forwarding.
    /// Default 100 = risk screening disabled (all scores pass).
    #[serde(default = "default_max_risk_score")]
    pub max_risk_score: u8,
}

fn default_max_risk_score() -> u8 { 100 }

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
    pub tsp_vault: crate::tsp_vault::TspVault,

    /// STAN Translation NAT table.
    /// Key:   Egress STAN assigned by the switch when forwarding to the upstream bank.
    /// Value: Original Ingress STAN received from the POS terminal / acquirer.
    ///
    /// Written at egress time (outbound), consumed (removed) when the bank 0110
    /// reply arrives. The Reaper GC removes stale entries on SLA timeout to
    /// prevent unbounded memory growth under sustained bank outages.
    pub stan_nat: DashMap<String, String>,

    /// Monotonically-incrementing counter for generating unique 6-digit egress
    /// STANs. Lock-free via AtomicU64 fetch_add. Wraps at 1,000,000 (6 digits).
    pub stan_counter: AtomicU64,
}

impl GlobalState {
    pub fn new() -> Self {
        Self {
            stip: ArcSwap::from_pointee(StipEngine { rules: HashMap::new() }),
            crdb: ArcSwap::from_pointee(CrdbEngine { cards: HashMap::new() }),
            router: ArcSwap::from_pointee(BinTrie::new()),
            node_health: DashMap::new(),
            egress_channels: DashMap::new(),
            tsp_vault: crate::tsp_vault::TspVault::new(),
            stan_nat: DashMap::new(),
            stan_counter: AtomicU64::new(0),
        }
    }
}
