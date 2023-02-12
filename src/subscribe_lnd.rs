use std::env;
use std::str::from_utf8;
use reqwest::Client;
use sqlx::{Pool, Postgres};
use sqlx::postgres::PgPoolOptions;
use crate::models::{Invoice, InvoiceResult, LightningChessResult, Transaction};

pub async fn update_settled_invoice(pool: &Pool<Postgres>, invoice: &Invoice) -> LightningChessResult<bool> {
    let mut tx = pool.begin().await?;
    println!("created tx");

    // look up in database
    let transaction = sqlx::query_as::<_,Transaction>( "SELECT * FROM lightningchess_transaction WHERE payment_addr=$1 FOR UPDATE")
        .bind(&invoice.payment_addr)
        .fetch_one(&mut tx).await?;
    println!("transaction: {}", serde_json::to_string(&transaction).unwrap());

    // update transaction table
    let amount = invoice.amt_paid_sat.parse::<i64>().unwrap();
    sqlx::query( "UPDATE lightningchess_transaction SET state='SETTLED', amount=$1 WHERE transaction_id=$2")
        .bind(&amount)
        .bind(&transaction.transaction_id)
        .execute(&mut tx).await?;
    println!("updated transaction");

    // update balance table
    sqlx::query( "INSERT INTO lightningchess_balance (username, balance) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET balance=lightningchess_balance.balance + $3 WHERE lightningchess_balance.username=$4")
        .bind(&transaction.username)
        .bind(&amount)
        .bind(&amount)
        .bind(&transaction.username)
        .execute(&mut tx).await?;
    println!("updated balance");

    // commit
    tx.commit().await?;
    println!("committed");
    Ok(true)
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
                    let res_bytes = res.chunk().await;
                    match res_bytes {
                        Ok(maybe_bytes) => {
                            match maybe_bytes {
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
                                            let db_update_result = update_settled_invoice(&pool, &invoice_result.result).await;
                                            match db_update_result {
                                                Ok(_) => (),
                                                Err(e) => println!("Error db update result {}", e)
                                            }
                                        }

                                        // after done reset chunky str
                                        invoice_str = "".to_string()
                                    }
                                },
                                None => {
                                    println!("No chonks");
                                    still_chunky = false;
                                }
                            }
                        }
                        Err(e) => {
                            println!("error chonking {}", e);
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