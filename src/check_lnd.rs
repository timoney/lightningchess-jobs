use std::env;
use std::str::from_utf8;
use reqwest::Client;
use sqlx::{Pool, Postgres};
use sqlx::postgres::PgPoolOptions;
use crate::models::{Invoice, InvoiceResult, Transaction};

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

pub async fn subscribe_invoices() {
    let db_url = env::var("DB_URL").unwrap();

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await.unwrap();

    loop {
        println!("Starting to subscribe to invoices!");
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