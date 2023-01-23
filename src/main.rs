use std::env;
use std::str::from_utf8;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Postgres};
use sqlx::postgres::PgPoolOptions;

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
    pub lichess_challenge_id: Option<String>
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

async fn update_settled_invoice(pool: &Pool<Postgres>, invoice: &Invoice) {
    // look up in database
    let transaction_result = sqlx::query_as::<_,Transaction>( "SELECT * FROM lightningchess_transaction WHERE payment_addr=$1")
        .bind(&invoice.payment_addr)
        .fetch_one(pool).await;

    let transaction = match transaction_result {
        Ok(t) => {
            println!("transaction: {}", serde_json::to_string(&t).unwrap());
            t
        },
        Err(e) => {
            println!("error: {}", e);
            return;
        }
    };

    // // update
    let tx_result = pool.begin().await;
    let mut tx = match tx_result {
        Ok(t) => t,
        Err(e) => {
            println!("error creating tx: {}", e);
            return;
        }
    };
    //
    // // update transaction table
    let amount = invoice.amt_paid_sat.parse::<i64>().unwrap();
    let updated_transaction = sqlx::query( "UPDATE lightningchess_transaction SET state='SETTLED', amount=$1 WHERE transaction_id=$2")
        .bind(&amount)
        .bind(&transaction.transaction_id)
        .execute(&mut tx).await;

    match updated_transaction {
        Ok(_) => println!("successfully updated_transaction transaction id {}", transaction.transaction_id),
        Err(e) => {
            println!("error updated_transaction transaction id : {}, {}", transaction.transaction_id, e);
            return;
        }
    }

    // update balance table
    let updated_balance = sqlx::query( "INSERT INTO lightningchess_balance (username, balance) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET balance=lightningchess_balance.balance + $3 WHERE lightningchess_balance.username=$4")
        .bind(&transaction.username)
        .bind(&amount)
        .bind(&amount)
        .bind(&transaction.username)
        .execute(&mut tx).await;

    match updated_balance {
        Ok(_) => println!("successfully updated_balance transaction id {}", transaction.transaction_id),
        Err(e) => {
            println!("error updated_balance transaction id : {}, {}", transaction.transaction_id, e);
            return;
        }
    }
    // commit
    let commit_result = tx.commit().await;
    match commit_result {
        Ok(_) => println!("successfully committed transaction id {}", transaction.transaction_id),
        Err(_) => {
            println!("error committing transaction id : {}", transaction.transaction_id);
            return;
        }
    }
}

async fn subscribe_invoices() {
    let db_url = env::var("DB_URL").unwrap();

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await.unwrap();

    loop {
        println!("Starting to subscribe to invoices!");
        println!("--------------------------------------------------------------------------------");
        println!("--------------------------------------------------------------------------------");
        println!("--------------------------------------------------------------------------------");
        println!("--------------------------------------------------------------------------------");
        let macaroon = env::var("LND_MACAROON").unwrap();

        let response = Client::new()
            .get(format!("https://lightningchess.m.voltageapp.io:8080/v1/invoices/subscribe"))
            .header("Grpc-Metadata-macaroon", macaroon)
            .send().await;

        match response {
            Ok(mut res) => {
                let mut still_chunky = true;
                let mut invoice_str = "".to_owned();
                while still_chunky {
                    let chunk_result = res.chunk().await;
                    match chunk_result {
                        Ok(maybe_bytes) => match maybe_bytes {
                            Some(bytes) => {
                                println!("bytes: {:?}", bytes);
                                let chunk = from_utf8(&bytes).unwrap();
                                println!("chunk: {}", chunk);
                                invoice_str.push_str(chunk);
                                println!("invoice str = {}", &invoice_str);
                                if chunk.ends_with("\n") {
                                    println!("End of invoice");
                                    let invoice_result: InvoiceResult = serde_json::from_str(&invoice_str).unwrap();
                                    // if result is settled update db
                                    if invoice_result.result.state == "SETTLED" {
                                        update_settled_invoice(&pool, &invoice_result.result).await;
                                    }

                                    // after done reset chunky str
                                    invoice_str = "".to_string()
                                }
                            },
                            None => {
                                println!("No chonks");
                                still_chunky = false;
                            }
                        },
                        Err(e) => {
                            println!("No more chunks error{}", e);
                            still_chunky = false;
                        }
                    }
                }
            },
            Err(e) => {
                println!("error in v1/invoices/subscribe :\n{}", e);
            }
        }
    }
}
#[tokio::main]
async fn main() {
    let profile = env::var("PROFILE").unwrap();
    if profile == "INVOICE" {
        subscribe_invoices().await
    } else if profile == "LICHESS" {
        println!("not yet implemented");
    } else {
        println!("not a valid profile. exiting...");
    }
}
