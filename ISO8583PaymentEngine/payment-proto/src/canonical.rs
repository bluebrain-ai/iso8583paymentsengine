use bytes::Bytes;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageClass {
    Authorization,
    Financial,
    ReversalAdvice,
    ReversalResponse,
    NetworkManagement,
    NetworkManagementResponse,
    FileUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionType {
    CashWithdrawal,
    BalanceInquiry,
    Purchase,
    Refund,
    Settlement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProcessingCode(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Stan(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LocalTime(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LocalDate(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rrn(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ResponseCode(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniversalPaymentEvent {
    pub message_class: MessageClass,
    pub transaction_type: TransactionType,
    pub mti: Bytes,
    pub fpan: Bytes, // Funding PAN
    pub dpan: Option<String>, // Device PAN
    pub is_tokenized: bool,
    pub tavv_cryptogram: Option<String>,
    pub processing_code: ProcessingCode,
    pub amount: u64,
    pub stan: Stan,
    pub local_time: LocalTime,
    pub local_date: LocalDate,
    pub rrn: Rrn,
    pub response_code: ResponseCode,
    pub acquirer_id: Bytes,
    pub pin_block: Bytes,
    /// Network risk score (0–100). Populated from a `RSK:NNN:` TLV prefix in
    /// Field 48 (Additional Private Data), as injected by the upstream network
    /// (e.g. Visa Advanced Authorization). Default 0 = no risk signal present.
    pub risk_score: u8,
    pub requires_instant_clearing: bool,
    pub domestic_settlement_data: Option<String>,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub original_data_elements: Option<String>,
    pub mac_data: Option<String>,
    pub is_reversal: bool,
}
