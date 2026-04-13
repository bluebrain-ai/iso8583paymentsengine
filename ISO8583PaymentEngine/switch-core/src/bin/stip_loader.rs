use serde::{Serialize, Deserialize};
use std::error::Error;
use csv::Reader;
use bincode;

use switch_core::context::CardProfile;

#[derive(Debug, serde::Deserialize)]
struct CsvRecord {
    pan: String,
    balance: i64,
    status: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        println!("Usage: stip_loader <csv_file>");
        return Ok(());
    }

    let file_path = &args[1];
    let mut rdr = Reader::from_path(file_path)?;

    // Open embedding directly matching Egress fallbacks natively
    let db = sled::open("stip.db")?;
    let mut count = 0;

    for result in rdr.deserialize() {
        let record: CsvRecord = result?;
        
        let profile = CardProfile {
            balance: record.balance,
            is_blocked: record.status.trim().eq_ignore_ascii_case("Blocked") || 
                        record.status.trim().eq_ignore_ascii_case("0"),
        };
        
        let encoded = bincode::serialize(&profile)?;
        db.insert(record.pan.as_bytes(), encoded)?;
        count += 1;
    }
    
    db.flush()?;
    println!("[INFO] STIP Load Complete: {} elements perfectly configured avoiding constraints.", count);
    Ok(())
}
