use std::{sync::Arc, time::Duration};

use monero_rpc::HashString;
use tokio::time::sleep;
use xmrapp::{PaymentStatus, XMRClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = XMRClient::<()>::new(
        String::from("http://127.0.0.1:38082"),
        String::from("stage.keys"),
        None
    ).await;
    // let mutex = Mutex::new(client);
    let arc = Arc::new(client);
    let c = arc.clone();
    let t = tokio::spawn(async move {
        loop {

            let amount = 1e9_f32.round() as u64;
            let (address, payment_id) = c.allocate_payment(amount).await.unwrap();
            let pid_str = HashString(payment_id);
            println!("Allocated payment with payment ID {} ({} pXMR):\n\n{}", pid_str, amount, address);
            // Check payment info
            let payment = xmrapp::XMRPayment {
                created_block_height: 1799376,
                created_timestamp: chrono::Utc::now(),
                status: xmrapp::PaymentStatus::Pending,
                amount_requested: amount,
                amount_confirmed: 0,
                amount_received: 0,
                info: None
            };
            let mut b: [u8; 8] = [0;8];
            hex::decode_to_slice("4435a6473cdc78bd", &mut b).unwrap();
            let payment_id = xmrapp::PaymentId(b);
            c.pending_payments.insert(payment_id, payment);
            let payment = c.poll_payment(payment_id).await.unwrap();
            println!("Payment info: {:#?}", payment);
            if payment.status == PaymentStatus::Confirmed {
                println!("\n========================");
                println!("PAYMENT CONFIRMED!");
                println!("========================\n");
            }
            sleep(Duration::from_secs(120)).await;

        }
    });
    t.await.unwrap();

    // println!("Getting addresses...");
    // let addresses = daemon.get_address(0, None).await?;
    // println!("Got addresses: {:#?}", addresses);
    // for (index, address) in addresses.addresses.iter().enumerate() {
    //     println!("Subaddress {} is {}", index, address.address);
    // }
    Ok(())
}