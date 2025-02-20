use std::{collections::{HashMap, VecDeque}, sync::{atomic::{AtomicU64, Ordering}, Mutex}};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use monero_rpc::{RpcClientBuilder, WalletClient};
pub use monero_rpc::monero::util::address::PaymentId;


#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaymentStatus {
    Expired,
    Pending,
    Received,
    Confirmed,
}
#[derive(Clone, Debug)]
pub struct XMRPayment<T: Clone> {
    pub created_timestamp: DateTime<Utc>,
    pub created_block_height: u64,
    pub status: PaymentStatus,
    /// Received amount in piconero (1e-12).
    pub amount_received: u64,
    /// Confirmed amount in piconero (1e-12).
    pub amount_confirmed: u64,
    /// Requested amount in piconero (1e-12).
    pub amount_requested: u64,
    /// Additional user information
    pub info: Option<T>,
}

pub struct XMRClient<T: Clone = ()> {
    pub wallet_client: WalletClient,
    pub pending_payments: DashMap<PaymentId, XMRPayment<T>>,
    pub current_block_height: AtomicU64,
    poll_queue: Mutex<VecDeque<PaymentId>>,
}
unsafe impl<T: Clone> Send for XMRClient<T> {}
unsafe impl<T: Clone> Sync for XMRClient<T> {}

// Let a payment last for 30 minutes or 15 blocks, whichever is longer


impl<T: Clone> XMRPayment<T> {
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


impl<T: Clone> XMRClient<T> {
    /// Creates a new XMRClient, a client to connect to a `monero-wallet-rpc` server.
    ///
    /// T will be the type of the additional info stored on each payment
    pub async fn new(
        rpc_address: String,
        wallet_file: String,
        wallet_password: Option<String>,
    ) -> Self {
        println!("Setting up the client...");
        let client = RpcClientBuilder::new()
            .build(rpc_address).unwrap();
        let daemon = client.wallet();
        println!("Opening wallet {} ...", wallet_file);
        daemon.open_wallet(wallet_file, wallet_password).await.expect("Could not open wallet");
        let height = daemon.get_height().await.unwrap();
        XMRClient {
            wallet_client: daemon,
            pending_payments: DashMap::new(),
            current_block_height: AtomicU64::from(height.get()),
            poll_queue: Mutex::new(VecDeque::new()),
        }
    }
}


impl<T: Clone> XMRClient<T> {

    /// Generates a payment address (integrated) by allocating a payment id.
    /// Returns the address as a string and the payment id.
    pub async fn allocate_payment(
        &self,
        amount_requested: u64,
        info: Option<T>
    ) -> anyhow::Result<(String, PaymentId)> {
        // get a new address if the payment id is already in use
        let (mut address, mut payment_id) = self.wallet_client.make_integrated_address(None, None).await?;
        let mut conflict = self.pending_payments.get(&payment_id.into());
        let current_block_height = self.current_block_height.load(Ordering::SeqCst);
        while let Some(ref conflict_value) = conflict {
            let payment_value = conflict_value.value();
            if payment_value.is_expired(current_block_height) {
                // We allow replacement of this payment
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
            amount_received: 0,
            amount_confirmed: 0,
            info,
        });
        Ok((address.to_string(), payment_id))
    }

    /// Returns the status of a payment as stored in the hashmap.
    /// Returns Expired if payment id is not found.
    ///
    /// Does **NOT** poll the RPC daemon for new changes - use `poll_payment` instead.
    pub fn query_payment(
        &self,
        payment_id: PaymentId
    ) -> Option<XMRPayment<T>> {
        let payment = self.pending_payments.get(&payment_id)?;
        Some(payment.value().clone())
    }

    /// Sets the `info` field of the relevant XMRPayment.
    /// Returns Some(()) on sucess and None if no XMRPayment was found.
    pub fn set_payment_info(
        &self,
        payment_id: PaymentId,
        payment_info: T
    ) -> Option<()> {
        let mut payment = self.pending_payments.get_mut(&payment_id)?;
        payment.info = Some(payment_info);
        Some(())
    }

    /// Enqueues the payment for polling.
    pub fn poll_payment(
        &self,
        payment_id: PaymentId
    ) {
        let mut lock = self.poll_queue.lock().unwrap();
        lock.push_back(payment_id);
    }

    /// Polls all enqueued payments. Should be run around every 5 seconds.
    ///
    /// Returns a Vec of changed payments.
    pub async fn poll_enqueued_payments(
        &self,
    ) -> anyhow::Result<Vec<XMRPayment<T>>> {
        let ids = {
            let mut lock = self.poll_queue.lock().unwrap();
            lock.drain(..).collect() // "extract" all values, clearing them
        };
        let current_bheight = self.wallet_client.get_height().await?.get();
        self.current_block_height.store(current_bheight, Ordering::SeqCst);
        let payments = self.wallet_client.get_bulk_payments(ids, current_bheight-1000).await?;
        #[derive(Default)]
        struct PaymentIdSum {
            received: u64,
            confirmed: u64,
        }
        // Collate the payments received by the wallet client
        let mut payment_id_sums: HashMap<PaymentId, PaymentIdSum> = HashMap::new();
        for payment in payments {
            let blocks_confirmed = current_bheight - payment.block_height;
            let received = payment.amount.as_pico();
            let mut confirmed = 0;
            if blocks_confirmed > 5 {
                confirmed = payment.amount.as_pico();
            }
            payment_id_sums.entry(payment.payment_id.0)
                .and_modify(|val| {
                    val.received += received;
                    val.confirmed += confirmed;
                })
                .or_insert(PaymentIdSum{
                    received,
                    confirmed,
                });
        }
        // Reflect the changes in self.pending_payments
        let mut changed_payments: Vec<XMRPayment<T>> = Vec::new();
        for (payment_id, sum) in payment_id_sums {
            let mut lock = match self.pending_payments.get_mut(&payment_id) {
                Some(val) => val,
                // Ignore cases of unregistered payment id
                None => continue,
            };
            lock.amount_confirmed = sum.confirmed;
            lock.amount_received = sum.received;
            if sum.confirmed >= lock.amount_requested {
                lock.status = PaymentStatus::Confirmed
            } else if sum.received >= lock.amount_requested {
                lock.status = PaymentStatus::Received
            }
            changed_payments.push(lock.value().clone());
        }
        Ok(changed_payments)
    }

    /// Polls the RPC daemon for progress one payment.
    /// Should not be used in most cases.
    pub async fn poll_payment_immediate(
        &self,
        payment_id: PaymentId
    ) -> anyhow::Result<XMRPayment<T>> {
        let mut pending_payment = match self.pending_payments.get_mut(&payment_id) {
            Some(value) => value,
            None => {
                return Err(anyhow::Error::msg(
                    format!("Could not find payment of id {}", payment_id)
                ));
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
        Ok(pending_payment.value().clone())
    }
}