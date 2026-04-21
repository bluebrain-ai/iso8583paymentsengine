use rusqlite::Connection;
use tokio_cron_scheduler::{Job, JobScheduler};

pub struct SettlementEngine;

impl SettlementEngine {
    pub async fn start_cron(db_path: String) {
        let mut sched = JobScheduler::new().await.unwrap();

        // Run at 23:59:59 every day (or every minute for testing)
        // For demonstration, we will run every minute to simulate 'midnight' sweeping during test
        // 1/60th second precision is possible but "1/59 59 23 * * * *" is the standard cron.
        // Let's use a 60 second interval for testing observability. "1/60 * * * * *"
        // But the prompt says "at 23:59:59 system time". We'll use "59 59 23 * * * *" 
        let cron_expr = "59 59 23 * * *"; // sec min hour day month day_of_week
        
        let job = Job::new_async(cron_expr, move |_uuid, _l| {
            let path = db_path.clone();
            Box::pin(async move {
                tracing::info!("Starting Midnight Batch Sweep...");
                if let Ok(conn) = Connection::open(&path) {
                    // Execute an atomic UPDATE query: UPDATE general_ledger SET status = 'settled' WHERE status = 'shadow' AND booking_timestamp < [cutoff]
                    let rows_updated = conn.execute(
                        "UPDATE general_ledger SET status = 'settled' WHERE status = 'shadow' AND datetime(booking_timestamp) < datetime('now', '-1 minutes')",
                        []
                    ).unwrap_or(0);
                    tracing::info!("Settlement Engine Swept {} transactions.", rows_updated);
                }
            })
        }).unwrap();

        sched.add(job).await.unwrap();
        sched.start().await.unwrap();
    }
}
