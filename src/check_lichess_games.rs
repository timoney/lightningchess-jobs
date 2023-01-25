use std::{env};
use tokio::time::{sleep, Duration};
use reqwest::{Client};
use sqlx::{Pool, Postgres};
use sqlx::postgres::PgPoolOptions;
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

async fn check(pool: &Pool<Postgres>) -> LightningChessResult<usize> {
    let admin = env::var("ADMIN_ACCOUNT").unwrap();

    // look up all the challenges in ACCEPTED status
    let challenges = sqlx::query_as::<_,Challenge>( "SELECT * FROM challenge WHERE STATUS='ACCEPTED' ORDER BY created_on DESC LIMIT 1000")
        .fetch_all(pool).await?;

    let num_challenges = challenges.len();
    println!("num_challenges: {}", num_challenges);

    // check in lichess if there are any updates
    for challenge in challenges.iter() {
        println!("processing challenge {}", serde_json::to_string(challenge).unwrap());

        let url = format!("https://lichess.org/game/export/{}", &challenge.lichess_challenge_id.as_ref().unwrap());
        let resp = Client::new()
            .get(url)
            .header("Accept", "application/json")
            .send().await?;

        if resp.status().as_u16() == 404 {
            println!("404 continuing...");
            // should consider this a draw after 30min
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
            println!("challenge not over yet {}", &challenge.lichess_challenge_id.as_ref().unwrap());
            continue;
        }

        let fee_per_person = calculate_fee_per_person(challenge);
        println!("fee per person{}", fee_per_person);
        let total_fee = fee_per_person * 2;

        let mut tx = pool.begin().await?;

        // pay admin
        let admin_ttype = "fee";
        let admin_detail = format!("fee from challenge {}", challenge.id);
        let admin_state = "SETTLED";
        sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state, lichess_challenge_id) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(&admin)
            .bind(admin_ttype)
            .bind(admin_detail)
            .bind(total_fee)
            .bind(admin_state)
            .bind(&challenge.lichess_challenge_id.as_ref().unwrap())
            .execute(&mut tx).await?;
        println!("insert admin transaction");

        sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
            .bind(total_fee)
            .bind(&admin)
            .execute(&mut tx).await?;
        println!("update admin balance");

        let winner = lichess_export_game_response.winner.get_or_insert("".to_string());
        if winner == "black" || winner == "white" {
            // pay money to winner
            let winner_username = get_winner_username(&challenge, winner);
            let winner_ttype = "winnings";
            let winner_detail = "";
            let winning_amt = (&challenge.sats.unwrap() * 2) - total_fee;
            let winner_state = "SETTLED";
            sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                .bind(&winner_username)
                .bind(winner_ttype)
                .bind(winner_detail)
                .bind(winning_amt)
                .bind(winner_state)
                .execute(&mut tx).await?;
            println!("insert winner transaction");

            sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                .bind(winning_amt)
                .bind(&winner_username)
                .execute(&mut tx).await?;
            println!("update winner balance");
        } else {
            // no winner so return money to both people
            let draw_ttype = "draw";
            let draw_detail = "initial sats amount minus 2% fee";
            let draw_amt = &challenge.sats.unwrap() - fee_per_person;
            let draw_state = "SETTLED";
            sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                .bind(&challenge.username)
                .bind(draw_ttype)
                .bind(draw_detail)
                .bind(draw_amt)
                .bind(draw_state)
                .execute(&mut tx).await?;
            println!("insert draw transaction 1");

            sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                .bind(draw_amt)
                .bind(&challenge.username)
                .execute(&mut tx).await?;
            println!("update draw balance 1");

            sqlx::query( "INSERT INTO lightningchess_transaction (username, ttype, detail, amount, state) VALUES ($1, $2, $3, $4, $5)")
                .bind(&challenge.opp_username)
                .bind(draw_ttype)
                .bind(draw_detail)
                .bind(draw_amt)
                .bind(draw_state)
                .execute(&mut tx).await?;
            println!("insert draw transaction 2");

            sqlx::query( "UPDATE lightningchess_balance set balance=balance + $1 WHERE username=$2")
                .bind(draw_amt)
                .bind(&challenge.opp_username)
                .execute(&mut tx).await?;
            println!("update draw balance 2");
        }

        // mark challenge as completed
        let status = "COMPLETED";
        sqlx::query_as::<_,Challenge>("UPDATE challenge SET status=$1 WHERE id=$2 RETURNING *")
            .bind(status)
            .bind(&challenge.id)
            .fetch_one(&mut tx).await?;
        println!("update challenge succeeded");

        // commit transaction
        tx.commit().await?;
        println!("committed");
    }

    Ok(num_challenges)
}

pub async fn check_lichess_games() -> () {
    println!("Starting to check lichess!");
    let db_url = env::var("DB_URL").unwrap();

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await.unwrap();

    let mut loop_count = 1;
    loop {
        println!("starting loop {}", loop_count);
        let check_result = check(&pool).await;
        let sleep_secs = match check_result {
            Ok(num_games_checked) => {
                println!("checked {} games", num_games_checked);
                if num_games_checked > 0 { 60 } else { 120 }
            },
            Err(e) => {
                println!("Error checking games {}", e);
                60
            }
        };
        // sleep longer if there are no currently open
        let duration = Duration::from_secs(sleep_secs);

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