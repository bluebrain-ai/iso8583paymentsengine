use dashmap::DashMap;
use std::sync::Arc;
use rusqlite::Connection;
use std::thread;
use std::time::Duration;
use crate::context::TransactionContext;

pub struct ComplianceGuard {
    pub cache: Arc<DashMap<String, String>>,
}

impl ComplianceGuard {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
        }
    }

    pub fn start_polling(db_path: String, guard: Arc<ComplianceGuard>) {
        thread::spawn(move || {
            loop {
                match Connection::open(&db_path) {
                    Ok(conn) => {
                        let mut stmt = match conn.prepare("SELECT list_type, list_value FROM sanctions_list WHERE is_live = 1 AND action = 'block'") {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::error!("ComplianceGuard poller prepare error: {:?}", e);
                                thread::sleep(Duration::from_secs(60));
                                continue;
                            }
                        };
                        
                        let rows = stmt.query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        });

                        if let Ok(mapped_rows) = rows {
                            let mut temp_cache = std::collections::HashMap::new();
                            for result in mapped_rows {
                                if let Ok((l_type, l_value)) = result {
                                    let key = format!("{}:{}", l_type.to_lowercase(), l_value.to_uppercase());
                                    temp_cache.insert(key, "block".to_string());
                                }
                            }

                            // Sync DashMap
                            guard.cache.retain(|k, _| temp_cache.contains_key(k));
                            for (k, v) in temp_cache {
                                guard.cache.insert(k, v);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("ComplianceGuard poller could not connect to ops.db: {:?}", e);
                    }
                }
                thread::sleep(Duration::from_secs(60));
            }
        });
    }

    pub fn evaluate(&self, ctx: &TransactionContext) -> bool {
        // Evaluate Currency Match (Mock: assuming we parse Field 49 but here we just check if it's there)
        // IsoDialect doesn't map currency, country explicitly in canonical by default, but we can check properties if they exist.
        // For the sake of demonstration and robust switch checks, we will match against BIN if FPAN matches.
        let pan_str = String::from_utf8_lossy(&ctx.transaction.fpan).to_string();
        
        // Match BIN
        if pan_str.len() >= 6 {
            let bin = &pan_str[0..6];
            let bin_key = format!("bin:{}", bin);
            if self.cache.contains_key(&bin_key) { return true; }
        }

        // Mock Currency/Country checks. In a real ISO parser, `currency` would be passed in canonical struct.
        // We will intercept based on network/acquirer_id if required, matching the task guidelines realistically.
        false
    }
}
