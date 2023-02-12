use std::error;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow};
use chrono::NaiveDateTime;


pub type LightningChessResult<T> = Result<T, Box<dyn error::Error>>;

fn default_string() -> String {
    "".to_string()
}
fn default_i32() -> i32 {
    0
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Transaction {
    #[serde(default = "default_i32")]
    pub transaction_id: i32,
    #[serde(default = "default_string")]
    pub username: String,
    pub ttype: String,
    pub detail: String,
    pub amount: i64,
    pub state: String,
    pub preimage: Option<String>, // base64 encoded
    pub payment_addr: Option<String>, // base64 encoded
    pub payment_request: Option<String>,
    pub payment_hash: Option<String>,
    pub lichess_challenge_id: Option<String>,
    pub created_on: Option<NaiveDateTime>
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct Challenge {
    #[serde(default = "default_i32")]
    pub id: i32,
    #[serde(default = "default_string")]
    pub username: String,
    pub time_limit: Option<i32>, // seconds
    pub opponent_time_limit: Option<i32>, // seconds
    pub increment: Option<i32>, // seconds
    pub color: Option<String>,
    pub sats: Option<i64>,
    pub opp_username: String,
    pub status: Option<String>,
    pub lichess_challenge_id: Option<String>,
    pub created_on: Option<NaiveDateTime>, // UTC
    pub expire_after: Option<i32> // seconds
}

#[derive(Serialize, Deserialize)]
pub struct InvoiceResult {
    pub result: Invoice
}

#[derive(Serialize, Deserialize)]
pub struct Invoice {
    pub memo: String,
    pub value: String,
    pub settled: bool,
    pub creation_date: String,
    pub settle_date: String,
    pub payment_request: String,
    pub payment_addr: String,
    pub expiry: String,
    pub amt_paid_sat: String,
    pub state: String
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LichessExportGameResponse {
    pub id: String,
    pub rated: bool,
    pub variant: String,
    pub speed: String,
    pub perf: String,
    pub status: String,
    pub winner: Option<String>
}

#[derive(Serialize, Deserialize)]
pub struct LookupInvoiceResponse {
    pub memo: String,
    pub value: String,
    pub settled: bool,
    pub creation_date: String,
    pub settle_date: String,
    pub payment_request: String,
    pub expiry: String,
    pub amt_paid_sat: String,
    pub state: String
}