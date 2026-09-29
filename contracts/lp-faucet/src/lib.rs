#![no_std]
#![feature(alloc_error_handler)]

use miden::{Asset, Felt, StorageValue, Word, component, component_storage, faucet};

#[component_storage]
struct LpFaucetStorage {
    #[storage(description = "supply")]
    supply: StorageValue<Word>,
}

#[component]
trait LpFaucet {
    fn issue(&mut self, asset: Asset) -> Asset;
    fn redeem(&mut self, asset: Asset);
    #[account_procedure]
    fn get_supply(&self) -> Felt;
}
#[component]
impl LpFaucet for LpFaucetStorage {
    fn issue(&mut self, asset: Asset) -> Asset {
        assert!(asset.is_fungible(), "LP asset must be fungible");
        let amount = asset.value[0].as_canonical_u64();
        assert!(amount > 0, "LP amount is zero");
        let current = self.supply.get()[0].as_canonical_u64();
        let next = current.checked_add(amount).expect("LP supply overflow");
        faucet::mint(asset);
        self.supply.set(Word::from([
            Felt::new(next).unwrap(),
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]));
        asset
    }
    fn redeem(&mut self, asset: Asset) {
        assert!(asset.is_fungible(), "LP asset must be fungible");
        let amount = asset.value[0].as_canonical_u64();
        let current = self.supply.get()[0].as_canonical_u64();
        assert!(current >= amount, "burn exceeds LP supply");
        faucet::burn(asset);
        let next = current - amount;
        self.supply.set(Word::from([
            Felt::new(next).unwrap(),
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]));
    }
    fn get_supply(&self) -> Felt {
        self.supply.get()[0]
    }
}
