use std::{env};
use std::collections::HashMap;
use chrono::NaiveDateTime;
use tokio::time::{sleep, Duration};
use reqwest::{Client};
use sqlx::{Error, Pool, Postgres};
use sqlx::postgres::{PgPoolOptions, PgQueryResult};
use chrono::prelude::Utc;
use crate::models::{Challenge, LightningChessResult, LichessExportGameResponse};

fn get_winner_username(challenge: &Challenge, winner: &str) -> String {
    // determine if user who created the challenge won
    let creator_won_black = winner == "black" && challenge.color.as_ref().unwrap() == "black";
    let creator_won_white = winner == "white" && challenge.color.as_ref().unwrap() == "white";
    return if creator_won_black || creator_won_white {
        &challenge.username
    } else {
        &challenge.opp_username
    }.to_string()
}

fn calculate_fee_per_person(challenge: &Challenge) -> i64 {
    let initial_fee: f64 = (challenge.sats.unwrap() as f64) * 0.02;
    initial_fee.floor() as i64
}

async fn insert_tx(tx: &mut sqlx::Transaction<'_, Postgres>,
                            username: &String,
                            ttype: &String,
                            detail: &String,
                            amount: i64,
                            state: &String,
                            lichess_challenge_id: &String) -> Result<PgQueryResult, Error>{
    sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state, lichess_challenge_id) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(username)
        .bind(ttype)
        .bind(detail)
        .bind(amount)
        .bind(state)
        .bind(lichess_challenge_id)
        .execute(tx).await
}

async fn add_to_balance(tx: &mut sqlx::Transaction<'_, Postgres>, username: &String, amt: i64) -> Result<PgQueryResult, Error> {
    sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
        .bind(amt)
        .bind(username)
        .execute(tx).await
}

async fn mark_challenge_completed(tx: &mut sqlx::Transaction<'_, Postgres>, challenge_id: i32) -> Result<Challenge, Error> {
    sqlx::query_as::<_,Challenge>("UPDATE challenge SET status='COMPLETED' WHERE id=$1 RETURNING *")
        .bind(challenge_id)
        .fetch_one(tx).await
}

async fn mark_challenge_expired(tx: &mut sqlx::Transaction<'_, Postgres>, challenge_id: i32) -> Result<Challenge, Error> {
    sqlx::query_as::<_,Challenge>("UPDATE challenge SET status='EXPIRED' WHERE id=$1 RETURNING *")
        .bind(challenge_id)
        .fetch_one(tx).await
}

async fn check(pool: &Pool<Postgres>, expired_challenges: &mut HashMap<String, i32>) -> LightningChessResult<usize> {
    let admin = env::var("ADMIN_ACCOUNT").unwrap();

    // look up all the challenges in ACCEPTED status
    let challenges = sqlx::query_as::<_,Challenge>( "SELECT * FROM challenge WHERE STATUS='ACCEPTED' ORDER BY created_on DESC LIMIT 1000")
        .fetch_all(pool).await?;

    let num_challenges = challenges.len();
    println!("num_challenges: {}", num_challenges);

    // check in lichess if there are any updates
    for challenge in challenges.iter() {
        println!("processing challenge {}", serde_json::to_string(challenge).unwrap());
        let lichess_challenge_id = challenge.lichess_challenge_id.as_ref().unwrap();
        let url = format!("https://lichess.org/game/export/{}", lichess_challenge_id);
        let resp = Client::new()
            .get(url)
            .header("Accept", "application/json")
            .send().await?;

        let fee_per_person = calculate_fee_per_person(challenge);
        let total_fee = fee_per_person * 2;

        let mut tx = pool.begin().await?;

        if resp.status().as_u16() == 404 {
            println!("404 game found for {}", lichess_challenge_id);
            let count = expired_challenges.entry(lichess_challenge_id.to_string()).or_insert(1);
            println!("count {count}");
            // if we get 404 for 30 cycles, mark as COMPLETED in draw
            if count > &mut 30 {
                println!("expired challenge {}. setting draw", lichess_challenge_id);
                let expired_ttype = "expired".to_string();
                let expired_detail = format!("sats returned for expired game");
                let expired_amt = challenge.sats.unwrap();
                let expired_state = "SETTLED".to_string();
                println!("insert expired transaction 1");
                insert_tx(&mut tx, &challenge.username, &expired_ttype, &expired_detail, expired_amt, &expired_state, lichess_challenge_id).await?;

                println!("update expired balance 1");
                add_to_balance(&mut tx, &challenge.username, expired_amt).await?;

                println!("insert expired transaction 2");
                insert_tx(&mut tx, &challenge.opp_username, &expired_ttype, &expired_detail, expired_amt, &expired_state, lichess_challenge_id).await?;

                println!("update expired balance 2");
                add_to_balance(&mut tx, &challenge.opp_username, expired_amt).await?;

                mark_challenge_completed(&mut tx, challenge.id).await?;
                println!("update challenge succeeded");

                tx.commit().await?;
                println!("committed");
            } else {
                *count += 1;
            }

            continue;
        }

        println!("Status: {}", resp.status());
        println!("Headers:\n{:#?}", resp.headers());

        let text = resp.text().await?;
        println!("raw text {}", text);

        let mut lichess_export_game_response: LichessExportGameResponse = serde_json::from_str(&text)?;
        println!("to domain type {lichess_export_game_response:?}");

        let challenge_lichess_result = lichess_export_game_response.status;
        if challenge_lichess_result == "created" || challenge_lichess_result == "started" {
            println!("challenge not over yet {}", lichess_challenge_id);
            continue;
        }

        // pay admin
        let admin_ttype = "fee".to_string();
        let admin_detail = format!("fee from challenge {}", challenge.id);
        let admin_state = "SETTLED".to_string();
        println!("insert admin transaction");
        insert_tx(&mut tx, &admin, &admin_ttype, &admin_detail, total_fee, &admin_state, lichess_challenge_id).await?;

        println!("update admin balance");
        add_to_balance(&mut tx, &admin, total_fee).await?;

        let winner = lichess_export_game_response.winner.get_or_insert("".to_string());
        if winner == "black" || winner == "white" {
            // pay money to winner
            let winner_username = get_winner_username(&challenge, winner);
            let winner_ttype = "winnings".to_string();
            let winner_detail = format!("lichess game https://lichess.org/{}", lichess_challenge_id);
            let winning_amt = (&challenge.sats.unwrap() * 2) - total_fee;
            let winner_state = "SETTLED".to_string();
            println!("insert winner transaction");
            insert_tx(&mut tx, &winner_username, &winner_ttype, &winner_detail, winning_amt, &winner_state, lichess_challenge_id).await?;

            println!("update winner balance");
            add_to_balance(&mut tx, &winner_username, winning_amt).await?;
        } else {
            // no winner so return money to both people
            let draw_ttype = "draw".to_string();
            let draw_detail = format!("lichess game https://lichess.org/{}. initial sats minus 2% fee", lichess_challenge_id);
            let draw_amt = &challenge.sats.unwrap() - fee_per_person;
            let draw_state = "SETTLED".to_string();
            println!("insert draw transaction 1");
            insert_tx(&mut tx, &challenge.username, &draw_ttype, &draw_detail, draw_amt, &draw_state, lichess_challenge_id).await?;

            println!("update draw balance 1");
            add_to_balance(&mut tx, &challenge.username, draw_amt).await?;

            println!("insert draw transaction 2");
            insert_tx(&mut tx, &challenge.opp_username, &draw_ttype, &draw_detail, draw_amt, &draw_state, lichess_challenge_id).await?;

            println!("update draw balance 2");
            add_to_balance(&mut tx, &challenge.opp_username, draw_amt).await?;
        }

        // mark challenge as completed
        mark_challenge_completed(&mut tx, challenge.id).await?;
        println!("update challenge succeeded");

        tx.commit().await?;
        println!("committed");
    }

    Ok(num_challenges)
}

async fn check_expired(pool: &Pool<Postgres>) -> LightningChessResult<usize> {
    // look up all the challenges in ACCEPTED status
    let challenges = sqlx::query_as::<_,Challenge>( "SELECT * FROM challenge WHERE STATUS='WAITING FOR ACCEPTANCE' ORDER BY created_on DESC LIMIT 1000")
        .fetch_all(pool).await?;

    let num_challenges = challenges.len();
    println!("num_challenges: {}", num_challenges);

    // unix time
    let current_seconds = Utc::now().timestamp();
    for challenge in challenges.iter() {
        println!("processing challenge {}", serde_json::to_string(challenge).unwrap());
        let mut tx = pool.begin().await?;
        let created_on: NaiveDateTime = challenge.created_on.unwrap();
        let challenge_seconds = created_on.timestamp();
        let diff_seconds = &current_seconds - challenge_seconds;
        let challenge_id= &challenge.id;
        println!("challenge: {challenge_id} diff_seconds: {diff_seconds}");
        // 30 min to seconds = 1800
        if diff_seconds > 1_800 {
            println!("setting challenge to expired");
            let expired_ttype = "expired".to_string();
            let expired_detail = format!("sats returned for expired game");
            let expired_amt = challenge.sats.unwrap();
            let expired_state = "SETTLED".to_string();
            let lichess_id = "none. expired".to_string();
            println!("insert expired transaction 1");
            insert_tx(&mut tx, &challenge.username, &expired_ttype, &expired_detail, expired_amt, &expired_state, &lichess_id).await?;

            println!("update expired balance 1");
            add_to_balance(&mut tx, &challenge.username, expired_amt).await?;

            println!("insert expired transaction 2");
            insert_tx(&mut tx, &challenge.opp_username, &expired_ttype, &expired_detail, expired_amt, &expired_state, &lichess_id).await?;

            println!("update expired balance 2");
            add_to_balance(&mut tx, &challenge.opp_username, expired_amt).await?;

            mark_challenge_expired(&mut tx, challenge.id).await?;
            println!("update challenge succeeded");

            tx.commit().await?;
            println!("committed");
        }
    }
    Ok(num_challenges)
}

pub async fn db_checks() -> () {
    println!("Starting db checks!");
    let db_url = env::var("DB_URL").unwrap();

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await.unwrap();

    let mut loop_count = 1;
    let mut expired_challenges: HashMap<String, i32> = HashMap::new();
    loop {
        println!("starting db checks loop {}", loop_count);
        // checks lichess to see if the game has finished
        let _check_result = check(&pool, &mut expired_challenges).await;

        // checks challenges to see if any have expired in 30min
        let _check_expired_result = check_expired(&pool).await;

        // makes sure that streaming didn't miss any invoices
        //let _check_invoices = reconcile(&pool).await;

        let duration = Duration::from_secs(60);

        println!("sleeping for {duration:?}");
        sleep(duration).await;
        loop_count = loop_count + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_challenge() -> Challenge {
        Challenge { id: 1,
            username: "user1".to_string(),
            time_limit: None,
            opponent_time_limit: None,
            increment: None,
            color: Some("white".to_string()),
            sats: Some(100),
            opp_username: "user2".to_string(),
            status: None,
            lichess_challenge_id: None,
            created_on: None,
            expire_after: None
        }
    }

    #[test]
    fn winner_creator_as_white() {
        let challenge = get_challenge();

        // creator won as white
        assert_eq!(get_winner_username(&challenge, "white"), "user1");
        // creator lost as white
        assert_eq!(get_winner_username(&challenge, "black"), "user2");
    }

    #[test]
    fn winner_creator_as_black() {
        let mut challenge = get_challenge();
        challenge.color = Some("black".to_string());

        // creator lost as black
        assert_eq!(get_winner_username(&challenge, "white"), "user2");
        // creator won as black
        assert_eq!(get_winner_username(&challenge, "black"), "user1");
    }

    #[test]
    fn calculate_fee_per_person_test() {
        let mut challenge = get_challenge();
        assert_eq!(calculate_fee_per_person(&challenge), 2);
        challenge.sats = Some(101);
        assert_eq!(calculate_fee_per_person(&challenge), 2);
        challenge.sats = Some(110);
        assert_eq!(calculate_fee_per_person(&challenge), 2);
        challenge.sats = Some(149);
        assert_eq!(calculate_fee_per_person(&challenge), 2);
        challenge.sats = Some(150);
        assert_eq!(calculate_fee_per_person(&challenge), 3);
        challenge.sats = Some(199);
        assert_eq!(calculate_fee_per_person(&challenge), 3);
        challenge.sats = Some(200);
        assert_eq!(calculate_fee_per_person(&challenge), 4);
    }
}