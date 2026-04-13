use tokio::net::TcpListener;
use tokio_util::codec::{FramedRead, FramedWrite};
use futures::{SinkExt, StreamExt};
use payment_proto::thales::{ThalesCodec, ThalesMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Mock Thales HSM started on 0.0.0.0:1500");
    let listener = TcpListener::bind("0.0.0.0:1500").await?;

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);
        
        tokio::spawn(async move {
            let (rx, tx) = stream.into_split();
            let mut framed_read = FramedRead::new(rx, ThalesCodec);
            let mut framed_write = FramedWrite::new(tx, ThalesCodec);

            while let Some(msg) = framed_read.next().await {
                match msg {
                    Ok(request) => {
                        println!("Received HSM command: {}", request.command);
                        let response = process_hsm_command(request);
                        if let Err(e) = framed_write.send(response).await {
                            println!("Failed to send HSM response: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        println!("Error decoding HSM request: {}", e);
                        break;
                    }
                }
            }
            println!("Connection closed: {}", addr);
        });
    }
}

fn process_hsm_command(mut msg: ThalesMessage) -> ThalesMessage {
    match msg.command.as_str() {
        "CA" => {
            // Translate PIN: reverse the pin payload hex
            let mut p = msg.payload.clone();
            let pin_block_start = 16 + 16 + 2; // SourceKey(16)+TargetKey(16)+MaxLen(2)
            if p.len() >= pin_block_start + 16 {
                let pin_block = &p[pin_block_start..pin_block_start+16];
                let reversed_pin: String = pin_block.chars().rev().collect();
                
                let mut resp_payload = p;
                resp_payload.replace_range(pin_block_start..pin_block_start+16, &reversed_pin);
                
                msg.command = "CB".to_string();
                // CB00[NewPINBlock]
                msg.payload = format!("00{}", reversed_pin);
            } else {
                msg.command = "CB".to_string();
                msg.payload = "01".to_string(); // Format error
            }
        }
        "DA" => {
            // Verify PIN
            if msg.payload.len() >= 16 + 16 + 2 + 12 + 16 + 4 {
                let pvv_start = msg.payload.len() - 4;
                let pvv = &msg.payload[pvv_start..];
                
                msg.command = "DB".to_string();
                if pvv == "9999" {
                    msg.payload = "05".to_string(); // Invalid PIN
                } else {
                    msg.payload = "00".to_string(); // Success
                }
            } else {
                msg.command = "DB".to_string();
                msg.payload = "01".to_string(); // Format error
            }
        }
        _ => {
            msg.command = msg.command.chars().next().unwrap().to_string() + "B"; // Generic error mapping XA to XB
            msg.payload = "15".to_string(); // Unknown command
        }
    }
    msg
}
