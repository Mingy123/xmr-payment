#![allow(dead_code)]

use std::{fmt::{self}, hash::{Hash, Hasher}, sync::atomic::{AtomicU64, Ordering}};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use monero_rpc::{RpcClientBuilder, WalletClient};


#[derive(Clone, Copy)]
pub struct PaymentID(pub [u8; 8]);
impl fmt::Display for PaymentID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl From<PaymentID> for monero_rpc::monero::util::address::PaymentId {
    fn from(value: PaymentID) -> Self {
        Self(value.0)
    }
}

impl From<monero_rpc::monero::util::address::PaymentId> for PaymentID {
    fn from(value: monero_rpc::monero::util::address::PaymentId) -> Self {
        Self(value.0)
    }
}

impl PartialEq for PaymentID {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for PaymentID {}

impl Hash for PaymentID {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub enum PaymentStatus {
    #[default]
    Expired,
    Pending,
    Received,
    Confirmed,
}
#[derive(Clone, Copy, Default, Debug)]
pub struct XMRPayment {
    pub created_timestamp: DateTime<Utc>,
    pub created_block_height: u64,
    pub status: PaymentStatus,
    /// Received amount in piconero (1e-12).
    pub amount_received: u64,
    /// Confirmed amount in piconero (1e-12).
    pub amount_confirmed: u64,
    /// Requested amount in piconero (1e-12).
    pub amount_requested: u64,
}

pub struct XMRClient {
    pub wallet_client: WalletClient,
    pub pending_payments: DashMap<PaymentID, XMRPayment>,
    pub current_block_height: AtomicU64,
}
unsafe impl Send for XMRClient {}
unsafe impl Sync for XMRClient {}

// Let a payment last for 30 minutes or 15 blocks, whichever is longer


impl XMRPayment {
    fn is_expired(&self, current_block_height: u64) -> bool {
        let time_passed = Utc::now() - self.created_timestamp;
        if time_passed.num_minutes() > 30 {
            return true;
        }
        if current_block_height - self.created_block_height > 15 {
            return true;
        }
        false
    }
}


impl XMRClient {

    pub async fn new(
        rpc_address: String,
        wallet_file: String,
        wallet_password: Option<String>,
    ) -> Self {
        println!("Setting up the client...");
        let client = RpcClientBuilder::new()
            .build(rpc_address).expect("Could not connect to RPC daemon.");
        let daemon = client.wallet();
        println!("Opening wallet {} ...", wallet_file);
        daemon.open_wallet(wallet_file, wallet_password).await.expect("Could not open wallet.");
        let height = daemon.get_height().await.unwrap();
        XMRClient {
            wallet_client: daemon,
            pending_payments: DashMap::new(),
            current_block_height: AtomicU64::from(height.get())
        }
    }

    /// Generates a payment address (integrated) by allocating a payment id.
    /// Returns the address as a string and the payment id.
    pub async fn allocate_payment(&self, amount_requested: u64) -> anyhow::Result<(String, PaymentID)> {
        // get a new address if the payment id is already in use
        let (mut address, mut payment_id) = self.wallet_client.make_integrated_address(None, None).await?;
        let mut conflict = self.pending_payments.get(&payment_id.into());
        let current_block_height = self.current_block_height.load(Ordering::SeqCst);
        while let Some(ref conflict_value) = conflict {
            let payment_value = conflict_value.value();
            if payment_value.is_expired(current_block_height) {
                break;
            }
            (address, payment_id) = self.wallet_client.make_integrated_address(None, None).await?;
            conflict = self.pending_payments.get(&payment_id.into());
        }
        // write it to the pending payments
        let payment_id = payment_id.into();
        self.pending_payments.insert(payment_id, XMRPayment {
            created_timestamp: Utc::now(),
            created_block_height: current_block_height,
            status: PaymentStatus::Pending,
            amount_requested,
            ..Default::default()
        });
        Ok((address.to_string(), payment_id))
    }

    /// Returns the status of a payment as stored in the hashmap.
    /// Returns Expired if payment id is not found.
    ///
    /// Does **NOT** poll the RPC daemon for new changes - use `poll_network` instead.
    pub fn query_payment(&self, payment_id: PaymentID) -> PaymentStatus {
        let payment = match self.pending_payments.get(&payment_id) {
            Some(v) => v,
            None => {
                return PaymentStatus::Expired;
            }
        };
        let value = payment.value();
        value.status
    }

    // TODO: Future Optimization: Store a list of payment IDs to query in 5 second time frames
    /// Polls the RPC daemon for progress on the payment.
    pub async fn poll_network(&self, payment_id: PaymentID) -> anyhow::Result<PaymentStatus> {
        // Function goal: look through the network for a fulfilled payment id
        let mut pending_payment = match self.pending_payments.get_mut(&payment_id) {
            Some(value) => value,
            None => {
                return Ok(PaymentStatus::Expired)
            }
        };
        let created_bheight = pending_payment.created_block_height;
        let payments = self.wallet_client.get_bulk_payments(
            vec![payment_id.into()],
            created_bheight
        ).await?;
        println!("Got payments for {} after {}:\n{:#?}", payment_id, created_bheight, payments);
        let mut received = 0;
        let mut confirmed = 0;
        for payment in payments.iter() {
            let blocks_confirmed = payment.block_height - created_bheight;
            received += payment.amount.as_pico();
            if blocks_confirmed > 5 {
                confirmed += payment.amount.as_pico();
            }
        }
        // write to dashmap
        pending_payment.amount_received = received;
        pending_payment.amount_confirmed = confirmed;
        if confirmed >= pending_payment.amount_requested {
            pending_payment.status = PaymentStatus::Confirmed
        } else if received >= pending_payment.amount_requested {
            pending_payment.status = PaymentStatus::Received
        }
        Ok(pending_payment.status)
    }
}