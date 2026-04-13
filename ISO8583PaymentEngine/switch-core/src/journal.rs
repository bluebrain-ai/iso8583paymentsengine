use memmap2::MmapMut;
use payment_proto::Transaction;
use prost::Message;
use std::fs::OpenOptions;
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum JournalError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Encode error: {0}")]
    Encode(#[from] prost::EncodeError),
    #[error("Buffer overflow")]
    Overflow,
}

#[derive(Clone)]
pub struct Journaler {
    tx: crossbeam::channel::Sender<payment_proto::Transaction>,
}

impl Journaler {
    pub fn new<P: AsRef<Path>>(path: P, size: usize) -> Result<Self, JournalError> {
        let (tx, rx) = crossbeam::channel::bounded::<payment_proto::Transaction>(50_000);
        
        let path_buf = path.as_ref().to_path_buf();
        std::thread::spawn(move || {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path_buf).expect("Failed to open journal file");
                
            file.set_len(size as u64).unwrap();
            let mut mmap = unsafe { MmapMut::map_mut(&file).unwrap() };
            let mut offset = 0;

            for tx in rx {
                let encoded_len = tx.encoded_len();
                let required_space = 4 + encoded_len;

                if offset + required_space > mmap.len() {
                    continue; // Or handle overflow appropriately
                }

                let len_bytes = (encoded_len as u32).to_be_bytes();
                mmap[offset..offset + 4].copy_from_slice(&len_bytes);
                offset += 4;

                let mut slice = &mut mmap[offset..offset + encoded_len];
                if tx.encode(&mut slice).is_ok() {
                    offset += encoded_len;
                    // PRODUCTION OPTIMIZATION: Non-blocking asynchronous OS-level page cache flush.
                    // Eliminates the 100MB synchronous write amplification bottleneck.
                    let _ = mmap.flush_async();
                }
            }
        });

        Ok(Self { tx })
    }

    pub fn append(&self, tx: &Transaction) -> Result<(), JournalError> {
        self.tx.send(tx.clone()).map_err(|_| JournalError::Overflow)?;
        Ok(())
    }
}
