use crossbeam::channel::Receiver;
use rusqlite::{params, Connection};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use payment_proto::canonical::UniversalPaymentEvent;
use dashmap::DashMap;

// 1. Fee Matrix Element
pub struct FeeRule {
    pub base_pct: f64,
    pub flat_fee_cents: i32,
    pub fx_markup_pct: f64,
}

// 2. Chart of Accounts Element
pub struct AccountMapping {
    pub vostro_account_id: String,
    pub nostro_account_id: String,
}

pub struct LedgerWorker {
    pub fee_matrix: Arc<DashMap<String, FeeRule>>,
    pub chart_of_accounts: Arc<DashMap<String, AccountMapping>>,
}

impl LedgerWorker {
    pub fn new() -> Self {
        Self {
            fee_matrix: Arc::new(DashMap::new()),
            chart_of_accounts: Arc::new(DashMap::new()),
        }
    }

    pub fn start_polling(&self, db_path: String) {
        let fee_cache = self.fee_matrix.clone();
        let coa_cache = self.chart_of_accounts.clone();
        let db_path_clone = db_path.clone();

        thread::spawn(move || {
            loop {
                if let Ok(conn) = Connection::open(&db_path_clone) {
                    // Update Fee Matrix Cache
                    if let Ok(mut stmt) = conn.prepare("SELECT mcc, bin_prefix, base_pct, flat_fee_cents, fx_markup_pct FROM fee_matrix WHERE is_live = 1") {
                        let _ = stmt.query_map([], |row| {
                            let mcc: Option<String> = row.get(0)?;
                            let bin: Option<String> = row.get(1)?;
                            
                            // fallback logic
                            let key = if let Some(m) = mcc { m } else if let Some(b) = bin { b } else { "DEFAULT".to_string() };

                            fee_cache.insert(key, FeeRule {
                                base_pct: row.get(2)?,
                                flat_fee_cents: row.get(3)?,
                                fx_markup_pct: row.get(4)?,
                            });
                            Ok(())
                        }).map(|iter| iter.count());
                    }

                    // Update Chart of Accounts Cache
                    if let Ok(mut stmt) = conn.prepare("SELECT institution_id, vostro_account_id, nostro_account_id FROM chart_of_accounts WHERE is_live = 1") {
                        let _ = stmt.query_map([], |row| {
                            let inst: String = row.get(0)?;
                            coa_cache.insert(inst, AccountMapping {
                                vostro_account_id: row.get(1)?,
                                nostro_account_id: row.get(2)?,
                            });
                            Ok(())
                        }).map(|iter| iter.count());
                    }
                }
                thread::sleep(Duration::from_secs(60));
            }
        });
    }

    pub fn start_worker(&self, db_path: String, rx: Receiver<UniversalPaymentEvent>) {
        let fee_cache = self.fee_matrix.clone();
        let coa_cache = self.chart_of_accounts.clone();

        thread::spawn(move || {
            let mut conn = match Connection::open(&db_path) {
                Ok(c) => {
                    println!("LEDGER WORKER SUCCESSFULLY CONNECTED TO DB AT {}", db_path);
                    let _ = c.execute_batch(
                        "PRAGMA journal_mode = WAL;
                         PRAGMA synchronous = NORMAL;
                         PRAGMA temp_store = MEMORY;"
                    );
                    c
                },
                Err(e) => {
                    eprintln!("FATAL: LEDGER WORKER COULD NOT CONNECT TO DB {}: {:?}", db_path, e);
                    panic!("Database binding failed");
                }
            };
            let mut batch_buffer = Vec::with_capacity(1000);
            let mut last_commit = std::time::Instant::now();

            loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(tx) => {
                        batch_buffer.push(tx);
                    }
                    Err(crossbeam::channel::RecvTimeoutError::Disconnected) => break,
                    Err(crossbeam::channel::RecvTimeoutError::Timeout) => {} // Just continue to flush
                }

                if batch_buffer.len() >= 1000 || (!batch_buffer.is_empty() && last_commit.elapsed().as_millis() >= 100) {
                    if let Ok(mut db_tx) = conn.transaction() {
                        {
                            let mut stmt = db_tx.prepare_cached(
                                "INSERT INTO general_ledger (trace_stan, trace_rrn, masked_pan, routing_bin, source_account_id, target_account_id, principal_amount, fee_amount, fx_rate, status, institution_id)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                            ).unwrap();
                            
                            for tx in batch_buffer.drain(..) {
                                let pan_str = String::from_utf8_lossy(&tx.fpan).to_string();
                                let fee_rule = if pan_str.len() >= 6 {
                                    if let Some(rule) = fee_cache.get(&pan_str[0..6]) {
                                        Some(rule)
                                    } else if let Some(def_rule) = fee_cache.get("DEFAULT") {
                                        Some(def_rule)
                                    } else { None }
                                } else { None };

                                let mut interchange_fee = 0;
                                let fx_rate = 1.00000;

                                if let Some(rule) = fee_rule {
                                    let amt = tx.amount as f64;
                                    let percentage_fee = amt * rule.value().base_pct;
                                    interchange_fee = percentage_fee as i64 + rule.value().flat_fee_cents as i64;
                                }

                                let inst_id = if pan_str.starts_with('4') {
                                    "CITI" 
                                } else if pan_str.starts_with('5') {
                                    "DANSKE_BANK" 
                                } else {
                                    "DEFAULT_INST"
                                };

                                let vostro = coa_cache.get(inst_id).map(|c| c.value().vostro_account_id.clone()).unwrap_or_else(|| "UNKNOWN".to_string());
                                let nostro = coa_cache.get(inst_id).map(|c| c.value().nostro_account_id.clone()).unwrap_or_else(|| "UNKNOWN".to_string());

                                let masked_pan = format!("{}****{}", &pan_str[0..6.min(pan_str.len())], &pan_str[pan_str.len().saturating_sub(4)..]);

                                let mut final_principal = tx.amount as i64;
                                let mut final_fee = interchange_fee;
                                let mut status = "shadow";

                                if tx.is_reversal {
                                    final_principal = -final_principal;
                                    final_fee = -final_fee;
                                    status = "reversed_offset_hold";
                                }

                                let result = stmt.execute(
                                    params![
                                        tx.stan.0.clone(),
                                        tx.rrn.0.clone(),
                                        masked_pan,
                                        if pan_str.len() >= 6 { &pan_str[0..6] } else { "000000" },
                                        vostro,
                                        nostro,
                                        final_principal,
                                        final_fee,
                                        fx_rate,
                                        status,
                                        inst_id
                                    ]
                                );
                                if let Err(e) = result {
                                    eprintln!(">>> LEDGER INSERT ERROR: {:?}", e);
                                }
                            }
                        }
                        if let Err(e) = db_tx.commit() {
                            eprintln!("FATAL: Failed to commit Ledger Batch: {:?}", e);
                        }
                    } else {
                        batch_buffer.clear(); // Drop if txn fails
                    }
                    last_commit = std::time::Instant::now();
                }
            }
        });
    }
}
