use prost::Message;
use std::fs::OpenOptions;
use std::io::Write;
use payment_proto::Transaction;
use memmap2::MmapOptions;
use csv::Writer;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let is_test = args.contains(&"--test".to_string());
    
    if is_test {
        println!("Test mode triggered.");
        return;
    }
    
    // Normal daemon implementation handles loops...
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            run_settlement_loop("test_wal.log").await;
        });
}

pub async fn run_settlement_loop(target_wal: &str) {
    let db = sled::open("tally.db").unwrap();
    let file_path = std::path::Path::new(target_wal);
    
    // Initial sleep offset for standard production tracking 
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(15 * 60)); 
    // At exactly midnight we would align a cron execution or check NaiveTime
    // For pure micro-batching we continuously sweep:

    loop {
        interval.tick().await;
        let file = match OpenOptions::new().read(true).open(file_path) {
            Ok(f) => f,
            Err(_) => continue, // WAL not created yet
        };

        let file_meta = file.metadata().unwrap();
        if file_meta.len() < 4 {
            continue;
        }

        let mmap = unsafe { MmapOptions::new().map(&file).unwrap() };
        
        let mut cursor = match db.get(b"cursor_offset") {
            Ok(Some(v)) => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&v);
                u64::from_be_bytes(bytes) as usize
            }
            _ => 0,
        };

        if cursor >= mmap.len() {
            continue;
        }

        while cursor + 4 <= mmap.len() {
            let mut len_bytes = [0u8; 4];
            len_bytes.copy_from_slice(&mmap[cursor..cursor+4]);
            let len = u32::from_be_bytes(len_bytes) as usize;

            if len == 0 {
                // Not cleanly synced EOF padding bound
                break;
            }

            if cursor + 4 + len > mmap.len() {
                // WAL truncated or incomplete flush
                break;
            }

            let tx_bytes = &mmap[cursor + 4..cursor + 4 + len];
            if let Ok(tx) = Transaction::decode(tx_bytes) {
                if tx.mti == "0100" && tx.response_code == "00" {
                    let mut current_total = match db.get(tx.acquirer_id.as_bytes()) {
                        Ok(Some(cached)) => {
                            let mut t = [0u8; 8];
                            t.copy_from_slice(&cached);
                            u64::from_be_bytes(t)
                        }
                        _ => 0,
                    };
                    
                    let mut current_count = match db.get(format!("{}_count", tx.acquirer_id).as_bytes()) {
                        Ok(Some(cached)) => {
                            let mut t = [0u8; 8];
                            t.copy_from_slice(&cached);
                            u64::from_be_bytes(t)
                        }
                        _ => 0,
                    };

                    current_total += tx.amount as u64;
                    current_count += 1;
                    
                    let _ = db.insert(tx.acquirer_id.as_bytes(), &current_total.to_be_bytes());
                    let _ = db.insert(format!("{}_count", tx.acquirer_id).as_bytes(), &current_count.to_be_bytes());
                }
            }
            cursor += 4 + len;
        }

        let _ = db.insert(b"cursor_offset", &(cursor as u64).to_be_bytes());
        let _ = db.flush();
    }
}

pub fn execute_cutoff(db: &sled::Db, output_file: &str) {
    let mut wtr = Writer::from_path(output_file).unwrap();
    wtr.write_record(&["Acquirer ID", "Total Count", "Total Settlement Amount"]).unwrap();

    let mut keys_to_reset = Vec::new();

    for row in db.iter() {
        if let Ok((key, val)) = row {
            let k_str = String::from_utf8_lossy(&key).to_string();
            if k_str == "cursor_offset" || k_str.ends_with("_count") {
                continue; // System bounds
            }
            
            let mut v_bytes = [0u8; 8];
            v_bytes.copy_from_slice(&val);
            let total_amt = u64::from_be_bytes(v_bytes);

            let mut count_val = 0;
            if let Ok(Some(cached)) = db.get(format!("{}_count", k_str).as_bytes()) {
                let mut c_bytes = [0u8; 8];
                c_bytes.copy_from_slice(&cached);
                count_val = u64::from_be_bytes(c_bytes);
            }

            wtr.write_record(&[
                k_str.clone(),
                count_val.to_string(),
                total_amt.to_string(),
            ]).unwrap();
            
            keys_to_reset.push(k_str.clone());
            keys_to_reset.push(format!("{}_count", k_str));
        }
    }

    wtr.flush().unwrap();
    
    // Clear targets starting identically for the next tracking day!
    for k in keys_to_reset {
        let _ = db.remove(k);
    }
    let _ = db.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use payment_proto::Transaction;
    use std::fs::OpenOptions;

    #[test]
    fn test_settlement_sweeps() {
        let _ = std::fs::remove_dir_all("test_tally.db");
        let _ = std::fs::remove_file("integration_test_wal.log");

        let log_path = "integration_test_wal.log";
        let size: usize = 10 * 1024 * 1024;

        // Execute exactly mimicking identical journal limits
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(log_path).unwrap();
        file.set_len(size as u64).unwrap();
        
        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
        let mut offset = 0;

        let db = sled::open("test_tally.db").unwrap();

        // Write 3 Transactions
        for _ in 0..3 {
            let tx = Transaction {
                mti: "0100".to_string(),
                pan: "41111111".to_string(),
                processing_code: "0".to_string(),
                amount: 100, // 3 * 100 = 300
                stan: "1".to_string(),
                rrn: "1".to_string(),
                response_code: "00".to_string(), // Approved
                acquirer_id: "BankA".to_string(),
                pin_block: "".to_string(),
                local_time: "120000".to_string(),
                local_date: "1231".to_string(),
            };
            
            let len = tx.encoded_len();
            mmap[offset..offset+4].copy_from_slice(&(len as u32).to_be_bytes());
            offset += 4;
            
            let mut slice = &mut mmap[offset..offset+len];
            tx.encode(&mut slice).unwrap();
            offset += len;
        }
        mmap.flush().unwrap();

        // 1. Run SWEEP
        let mut cursor = 0;
        sweep_logic(&db, &mmap[..], &mut cursor);
        
        // Assert BankA has 300
        let cached = db.get("BankA").unwrap().unwrap();
        let mut t = [0u8; 8];
        t.copy_from_slice(&cached);
        assert_eq!(u64::from_be_bytes(t), 300);

        // Write 2 more
        for _ in 0..2 {
            let tx = Transaction {
                mti: "0100".to_string(),
                pan: "41111111".to_string(),
                processing_code: "0".to_string(),
                amount: 50, // 2 * 50 = 100 (Total 400)
                stan: "2".to_string(),
                rrn: "2".to_string(),
                response_code: "00".to_string(), 
                acquirer_id: "BankA".to_string(),
                pin_block: "".to_string(),
                local_time: "120000".to_string(),
                local_date: "1231".to_string(),
            };
            
            let len = tx.encoded_len();
            mmap[offset..offset+4].copy_from_slice(&(len as u32).to_be_bytes());
            offset += 4;
            let mut slice = &mut mmap[offset..offset+len];
            tx.encode(&mut slice).unwrap();
            offset += len;
        }

        // 2. SWEEP again
        sweep_logic(&db, &mmap[..], &mut cursor);
        
        let cached2 = db.get("BankA").unwrap().unwrap();
        let mut t2 = [0u8; 8];
        t2.copy_from_slice(&cached2);
        assert_eq!(u64::from_be_bytes(t2), 400); // 300 + 100!

        // Trigger Cutover
        execute_cutoff(&db, "test_cutoff.csv");

        let csv_contents = std::fs::read_to_string("test_cutoff.csv").unwrap();
        assert!(csv_contents.contains("BankA,5,400"));

        let _ = std::fs::remove_dir_all("test_tally.db");
        let _ = std::fs::remove_file("integration_test_wal.log");
        let _ = std::fs::remove_file("test_cutoff.csv");
    }

    // Extracted loop logic matching identical core arrays avoiding macro overlaps natively
    fn sweep_logic(db: &sled::Db, mmap: &[u8], cursor: &mut usize) {
        let cached_cur = match db.get(b"cursor_offset") {
            Ok(Some(v)) => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&v);
                u64::from_be_bytes(bytes) as usize
            }
            _ => 0,
        };
        *cursor = cached_cur;

        while *cursor + 4 <= mmap.len() {
            let mut len_bytes = [0u8; 4];
            len_bytes.copy_from_slice(&mmap[*cursor..*cursor+4]);
            let len = u32::from_be_bytes(len_bytes) as usize;

            if len == 0 { break; }
            if *cursor + 4 + len > mmap.len() { break; }

            let tx_bytes = &mmap[*cursor + 4..*cursor + 4 + len];
            if let Ok(tx) = Transaction::decode(tx_bytes) {
                if tx.mti == "0100" && tx.response_code == "00" {
                    let mut current_total = match db.get(tx.acquirer_id.as_bytes()) {
                        Ok(Some(c)) => {
                            let mut t = [0u8; 8];
                            t.copy_from_slice(&c);
                            u64::from_be_bytes(t)
                        }
                        _ => 0,
                    };
                    let mut current_count = match db.get(format!("{}_count", tx.acquirer_id).as_bytes()) {
                        Ok(Some(c)) => {
                            let mut t = [0u8; 8];
                            t.copy_from_slice(&c);
                            u64::from_be_bytes(t)
                        }
                        _ => 0,
                    };

                    current_total += tx.amount as u64;
                    current_count += 1;
                    
                    let _ = db.insert(tx.acquirer_id.as_bytes(), &current_total.to_be_bytes());
                    let _ = db.insert(format!("{}_count", tx.acquirer_id).as_bytes(), &current_count.to_be_bytes());
                }
            }
            *cursor += 4 + len;
        }

        let _ = db.insert(b"cursor_offset", &(*cursor as u64).to_be_bytes());
        let _ = db.flush();
    }
}
