// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

pub mod amm;
pub mod error;
pub mod utils;

use miden::{AccountId, StorageValue, component, component_storage};

use crate::amm::{fees::Fees, reserves::Reserves, token::Tokens};

/// Storage layout for the account component (empty).
#[component_storage]
struct Univ2ammStorage {
    #[storage(description = "AccountIds of token0 and token1")]
    tokens: StorageValue<Tokens>,
    #[storage(description = "Reserves of token0, token1 and total lp supply")]
    reserves: StorageValue<Reserves>,
    #[storage(description = "Fee beneficiary and the fee amount")]
    fees: StorageValue<Fees>,
    #[storage(description = "AccountId that is permitted to make admin changes")]
    admin: StorageValue<AccountId>,
}

impl Univ2ammStorage {}

#[component]
trait Univ2amm {
    // // Views
    fn token0(&self) -> AccountId;
    fn token1(&self) -> AccountId;
    fn get_reserves(&self) -> Reserves;
}

#[component]
impl Univ2amm for Univ2ammStorage {
    fn token0(&self) -> AccountId {
        self.tokens.get().token0()
    }
    fn token1(&self) -> AccountId {
        self.tokens.get().token1()
    }
    fn get_reserves(&self) -> Reserves {
        self.reserves.get()
    }
}
