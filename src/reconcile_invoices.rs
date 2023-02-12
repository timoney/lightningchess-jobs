use std::env;
use chrono::{NaiveDateTime, Utc};
use reqwest::Client;
use sqlx::{Pool, Postgres};
use crate::models::{LightningChessResult, LookupInvoiceResponse, Transaction};
// this serves as a backup to the streaming
// if the invoice streaming goes down, this should be able to reconcile invoices
pub async fn _reconcile(pool: &Pool<Postgres>) -> LightningChessResult<usize> {
    // look up all the transactions that are in OPEN status
    let transactions = sqlx::query_as::<_, Transaction>("SELECT * FROM lightningchess_transaction WHERE state='OPEN' LIMIT 1000")
        .fetch_all(pool).await?;

    let num_transactions = transactions.len();
    println!("num_transactions: {}", num_transactions);

    // unix time
    let current_seconds = Utc::now().timestamp();
    let macaroon = env::var("LND_MACAROON").unwrap();
    for transaction in transactions.iter() {
        println!("processing transaction {}", serde_json::to_string(transaction).unwrap());
        let mut _tx = pool.begin().await?;
        let created_on: NaiveDateTime = transaction.created_on.unwrap();
        let transaction_seconds = created_on.timestamp();
        let diff_seconds = &current_seconds - transaction_seconds;
        let transaction_id = &transaction.transaction_id;
        println!("transaction: {transaction_id} diff_seconds: {diff_seconds}");
        // 30 min to seconds = 1800. this is default invoice expiry time. add a little bit to not interfere with streaming
        if diff_seconds > 2_000 {
            // check lnd to see status
            let payment_addr = transaction.payment_addr.as_ref().unwrap();
            let base64_decoded_bytes = base64::decode(payment_addr).unwrap();
            let base64_url_safe_encoded = base64::encode_config(base64_decoded_bytes, base64::URL_SAFE);
            println!("base64_url_safe_encoded: {}", base64_url_safe_encoded);
            let response = Client::new()
                .get(format!("https://lightningchess.m.voltageapp.io:8080/v2/invoices/lookup?payment_addr={}", base64_url_safe_encoded))
                .header("Grpc-Metadata-macaroon", &macaroon)
                .send().await;

            match response {
                Ok(res) => {
                    //println!("Status: {}", res.status());
                    //println!("Headers:\n{:#?}", res.headers());
                    let text = res.text().await;
                    match text {
                        Ok(text) => {
                            //println!("text: {}", text);
                            let lookup_invoice_response: LookupInvoiceResponse = serde_json::from_str(&text).unwrap();
                            println!("lookupInvoiceResponse: {}", serde_json::to_string(&lookup_invoice_response).unwrap());
                            // if expired then set to expired

                            // if paid then pay out
                        }
                        Err(e) => {
                            println!("error in text() :\n{}", e);
                        }
                    }
                },
                Err(e) => {
                    println!("error from lnd lookup_invoice\n{}", e);
                }
            };
        }
    }
    Ok(num_transactions)
}
