use std::{env};
use tokio::time::{sleep, Duration};
use reqwest::{Client};
use sqlx::postgres::PgPoolOptions;
use crate::models::{Challenge, LichessExportGameResponse};

pub async fn check_lichess_games() {
    println!("Starting to check lichess!");
    let db_url = env::var("DB_URL").unwrap();

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await.unwrap();

    let admin_result = env::var("ADMIN_ACCOUNT");
    let admin = match admin_result {
        Ok(a) => a,
        Err(e) => {
            println!("error getting admin account: {}", e);
            return;
        }
    };

    loop {
        // 1. look up all the challenges in ACCEPTED status
        let challenges_result = sqlx::query_as::<_,Challenge>( "SELECT * FROM challenge WHERE STATUS='ACCEPTED' ORDER BY created_on DESC LIMIT 1000")
            .fetch_all(&pool).await;

        let challenges = match challenges_result {
            Ok(cs) => cs,
            Err(e) => {
                println!("error getting challenges: {}", e);
                return
            }
        };

        // 2. check in lichess if there are any updates
        for challenge in challenges.iter() {
            println!("processing challenge {}", serde_json::to_string(challenge).unwrap());

            let url = format!("https://lichess.org/game/export/{}", &challenge.lichess_challenge_id.as_ref().unwrap());
            let resp = Client::new()
                .get(url)
                .header("Accept", "application/json")
                .send().await;

            let mut lichess_export_game_response: LichessExportGameResponse = match resp {
                Ok(res) => {
                    if res.status().as_u16() == 404 {
                        println!("404 continuing...");
                        continue;
                    }
                    println!("Status: {}", res.status());
                    println!("Headers:\n{:#?}", res.headers());

                    let text = res.text().await;
                    match text {
                        Ok(text) => {
                            println!("text!: {}", text);
                            serde_json::from_str(&text).unwrap()
                        }
                        Err(e) => {
                            println!("error getting game on lichess text(): {}", e);
                            return;
                        }
                    }
                },
                Err(e) => {
                    println!("error getting game on lichess : {}", e);
                    return;
                }
            };

            let challenge_lichess_result = lichess_export_game_response.status;
            if challenge_lichess_result == "created" || challenge_lichess_result == "started" {
                println!("challenge not over yet {}", &challenge.lichess_challenge_id.as_ref().unwrap());
                continue;
            }

            // determine fee
            let initial_fee: f64 = (challenge.sats.unwrap() as f64) * 0.02;
            let rounded_down = initial_fee.floor() as i64;
            // make even
            let fee = rounded_down - rounded_down % 2;

            let tx_result = pool.begin().await;
            let mut tx = match tx_result {
                Ok(t) => t,
                Err(e) => {
                    println!("error creating tx: {}", e);
                    return;
                }
            };

            // pay admin
            let admin_ttype = "fee";
            let admin_detail = format!("fee from challenge {}", challenge.id);
            let admin_state = "SETTLED";
            let admin_transaction_result = sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state, lichess_challenge_id) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(&admin)
                .bind(admin_ttype)
                .bind(admin_detail)
                .bind(fee)
                .bind(admin_state)
                .bind(&challenge.lichess_challenge_id.as_ref().unwrap())
                .execute(&mut tx).await;

            match admin_transaction_result {
                Ok(_) => println!("insert transaction successfully"),
                Err(e) => {
                    println!("insert transaction failed {}", e);
                    return;
                }
            }

            let admin_balance = sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                .bind(fee)
                .bind(&admin)
                .execute(&mut tx).await;

            match admin_balance {
                Ok(_) => println!("successfully payed admin"),
                Err(e) => {
                    println!("error paying admin admin_transaction transaction{}", e);
                    return;
                }
            }

            let winner = lichess_export_game_response.winner.get_or_insert("".to_string());
            if winner == "black" || winner == "white" {
                // pay money to winner
                let winner_username = if challenge.color.as_ref().unwrap() == "black" { &challenge.username } else { &challenge.opp_username };
                let winner_ttype = "winnings";
                let winner_detail = "";
                let winning_amt = (&challenge.sats.unwrap() * 2) - fee;
                let winner_state = "SETTLED";
                let winner_transaction_result = sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                    .bind(winner_username)
                    .bind(winner_ttype)
                    .bind(winner_detail)
                    .bind(winning_amt)
                    .bind(winner_state)
                    .execute(&mut tx).await;

                match winner_transaction_result {
                    Ok(_) => println!("insert transaction successfully"),
                    Err(e) => {
                        println!("insert transaction failed {}", e);
                        return;
                    }
                }

                let winner_balance = sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                    .bind(winning_amt)
                    .bind(winner_username)
                    .execute(&mut tx).await;

                match winner_balance {
                    Ok(_) => println!("successfully payed admin"),
                    Err(e) => {
                        println!("error paying admin admin_transaction transaction{}", e);
                        return;
                    }
                }
            } else {
                // no winner so return money to both people
                let draw_ttype = "draw";
                let draw_detail = "initial sats amount minus 2% fee";
                let draw_amt = &challenge.sats.unwrap() - (fee / 2);
                let draw_state = "SETTLED";
                let draw_transaction_result = sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                    .bind(&challenge.username)
                    .bind(draw_ttype)
                    .bind(draw_detail)
                    .bind(draw_amt)
                    .bind(draw_state)
                    .execute(&mut tx).await;

                match draw_transaction_result {
                    Ok(_) => println!("insert transaction successfully"),
                    Err(e) => {
                        println!("insert transaction failed {}", e);
                        return;
                    }
                }

                let draw_balance = sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                    .bind(draw_amt)
                    .bind(&challenge.username)
                    .execute(&mut tx).await;

                match draw_balance {
                    Ok(_) => println!("successfully payed draw 1"),
                    Err(e) => {
                        println!("error paying draw 1 balance transaction{}", e);
                        return;
                    }
                }

                let draw_transaction_result2 = sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                    .bind(&challenge.opp_username)
                    .bind(draw_ttype)
                    .bind(draw_detail)
                    .bind(draw_amt)
                    .bind(draw_state)
                    .execute(&mut tx).await;

                match draw_transaction_result2 {
                    Ok(_) => println!("insert transaction successfully"),
                    Err(e) => {
                        println!("insert transaction failed {}", e);
                        return;
                    }
                }

                let draw_balance2 = sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                    .bind(draw_amt)
                    .bind(&challenge.opp_username)
                    .execute(&mut tx).await;

                match draw_balance2 {
                    Ok(_) => println!("successfully payed draw 2"),
                    Err(e) => {
                        println!("error paying draw 2 balance transaction{}", e);
                        return;
                    }
                }
            }

            // mark challenge as completed
            // update challenge in db
            let status = "COMPLETED";
            let pg_query_result = sqlx::query_as::<_,Challenge>("UPDATE challenge SET status=$1 WHERE id=$2 RETURNING *")
                .bind(status)
                .bind(&challenge.id)
                .fetch_one(&mut tx).await;

            match pg_query_result {
                Ok(_) => println!("update challenge succeeded"),
                Err(e) => {
                    println!("update challenge failed: {}", e);
                    return;
                }
            };

            // commit transaction, return challenge
            let commit_result = tx.commit().await;
            match commit_result {
                Ok(_) => println!("successfully committed"),
                Err(e) => println!("error committing: {}", e)
            }
        }

        // sleep longer if there are no currently open
        let duration = if challenges.len() == 0 {
            Duration::from_secs(120)
        } else {
            Duration::from_secs(60)
        };

        println!("sleeping for {duration:?}");
        sleep(duration).await;
    }
}