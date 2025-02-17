use std::sync::Arc;

use hex::FromHex;
use monero_rpc::{monero::util::address::PaymentId, HashString};
use tokio::time::sleep;
use xmrapp::{XMRClient, XMRPayment};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = XMRClient::new(
        String::from("http://127.0.0.1:38082"),
        String::from("stage.keys"),
        None
    ).await;
    // let mutex = Mutex::new(client);
    let arc = Arc::new(client);
    let c = arc.clone();
    let t = tokio::spawn(async move {
        let amount = 1e9_f32.round() as u64;
        let (address, payment_id) = c.allocate_payment(amount).await.unwrap();
        let pid_str = HashString(payment_id);
        println!("Allocated payment with payment ID {} ({} pXMR):\n\n{}", pid_str, amount, address);
        // sleep(Duration::from_secs(120)).await;
        // Check payment info
        let payment = XMRPayment {
            created_block_height: 1799376,
            created_timestamp: chrono::Utc::now(),
            status: xmrapp::PaymentStatus::Pending,
            amount_requested: amount,
            ..Default::default()
        };
        let id = PaymentId::from_hex("4435a6473cdc78bd").unwrap();
        c.pending_payments.insert(id, payment);
        c.poll_network(id).await.unwrap();
        let value = c.pending_payments.get(&id).unwrap();
        println!("Payment info: {:#?}", value.value());
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