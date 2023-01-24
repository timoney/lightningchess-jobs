mod check_lnd;
mod check_lichess_games;
mod models;

use crate::check_lnd::subscribe_invoices;
use crate::check_lichess_games::check_lichess_games;

#[tokio::main]
async fn main() {
    let subscribe_task = tokio::spawn(async move {
        subscribe_invoices().await
    });

    check_lichess_games().await;

    subscribe_task.await.unwrap();
}
