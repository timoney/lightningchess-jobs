mod subscribe_lnd;
mod db_checks;
mod reconcile_invoices;
mod models;

use crate::subscribe_lnd::subscribe_invoices;
use crate::db_checks::db_checks;

#[tokio::main]
async fn main() {
    let subscribe_task = tokio::spawn(async move {
        subscribe_invoices().await
    });

    db_checks().await;

    subscribe_task.await.unwrap();
}
