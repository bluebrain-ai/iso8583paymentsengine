use payment_proto::Transaction;
use tokio::sync::oneshot;
use std::time::Instant;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct CardProfile {
    pub balance: i64,
    pub is_blocked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
pub enum NetworkState {
    HSMPending,
    UpstreamSent,
    Reversed,
    Completed,
}

pub struct ActiveTransaction {
    pub created_at: Instant,
    pub state: NetworkState,
    pub responder: Option<oneshot::Sender<payment_proto::canonical::UniversalPaymentEvent>>,
    pub transaction_data: payment_proto::Transaction,
    /// The egress STAN assigned by the NAT when this transaction was forwarded
    /// to an external node. `None` for CRDB-routed or STIP-answered transactions
    /// (no upstream bank involved). Used by the Reaper to GC `GlobalState::stan_nat`
    /// on SLA timeout without a full-map scan.
    pub egress_stan: Option<String>,
}

pub struct TransactionContext {
    pub transaction: payment_proto::canonical::UniversalPaymentEvent,
    pub responder: Option<oneshot::Sender<payment_proto::canonical::UniversalPaymentEvent>>,
}
