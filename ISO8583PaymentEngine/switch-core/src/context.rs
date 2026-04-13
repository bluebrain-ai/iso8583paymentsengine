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
    pub responder: Option<oneshot::Sender<payment_proto::canonical::CanonicalTransaction>>,
    pub transaction_data: payment_proto::Transaction,
}

pub struct TransactionContext {
    pub transaction: payment_proto::canonical::CanonicalTransaction,
    pub responder: Option<oneshot::Sender<payment_proto::canonical::CanonicalTransaction>>,
}
