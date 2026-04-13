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
pub struct CanonicalTransaction {
    pub message_class: MessageClass,
    pub transaction_type: TransactionType,
    pub mti: Bytes,
    pub pan: Bytes,
    pub processing_code: ProcessingCode,
    pub amount: u64,
    pub stan: Stan,
    pub local_time: LocalTime,
    pub local_date: LocalDate,
    pub rrn: Rrn,
    pub response_code: ResponseCode,
    pub acquirer_id: Bytes,
    pub pin_block: Bytes,
}
