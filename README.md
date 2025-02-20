# XMR_PAYMENT

This library runs a client to the Monero Wallet RPC
([docs](https://docs.getmonero.org/rpc-library/wallet-rpc/)).  
Powered by async rust, this library is designed to be simple, efficient and safe.

## Features

This library provides an interface to allocate a payment address for a user using Monero's integrated address feature.  
Information on these addresses are stored in a hashmap of `PaymentId` to `XMRPayment`, which contains:
- Status
- Created timestamp/block height
- Amount requested/received/confirmed
- Any additional information you may want to attach to the payment.

Additionally, the following functions are available for polling the network:
- Immediate poll of a singular payment id
- Enqueue polling of a payment id
- Bulk polling of all enqueued payment ids

Bulk polling is recommended to reduce the load on networking with monero-wallet-rpc.

## Using this library

1. Install `monero-wallet-rpc`: Install the general Monero package via your package manager,
or from [getmonero.org](https://www.getmonero.org/downloads/#cli)
2. Run `monero-wallet-rpc` as follows:  
`monero-wallet-rpc --daemon-address <http://address:port> --wallet-dir /path/to/wallet/dir --rpc-bind-port 38082 --disable-rpc-login`
3. Add this library: Add the following line in your `Cargo.toml`:  
`monero-rpc = { git = "https://github.com/Mingy123/xmr-payment }`
4. Create the XMRClient:  
```
    let client = XMRClient::<T>::new(
        String::from("http://127.0.0.1:38082"),
        String::from("wallet.keys"),
        None
    ).await;
```
T is the type to be attached to each payment info (`XMRPayment`). Use `()` for nothing.