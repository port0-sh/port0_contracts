// Do not link against libstd (i.e. anything defined in `std::`)
#![no_std]
#![feature(alloc_error_handler)]

use miden::{Asset, Felt, StorageValue, Word, component, component_storage, faucet};

#[component_storage]
struct LpFaucetStorage {
    #[storage(description = "supply")]
    supply: StorageValue<Word>,
}

/// Faucet surface. Only the pool component should call `issue` and `redeem`.
#[component]
trait LpFaucet {
    #[account_procedure]
    fn issue(&mut self, asset: Asset);
    #[account_procedure]
    fn redeem(&mut self, asset: Asset);
    #[account_procedure]
    fn get_supply(&self) -> Felt;
}
#[component]
impl LpFaucet for LpFaucetStorage {
    fn issue(&mut self, asset: Asset) {
        assert!(asset.is_fungible(), "LP asset must be fungible");
        let amount = asset.value[0].as_canonical_u64();
        assert!(amount > 0, "LP amount is zero");
        let current = self.supply.get()[0].as_canonical_u64();
        let next = current.checked_add(amount).expect("LP supply overflow");
        // Records the asset as issued by this account. Does not put it in the vault.
        faucet::mint(asset);
        self.supply.set(Word::from([
            Felt::new(next).unwrap(),
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]));
    }
    fn redeem(&mut self, asset: Asset) {
        assert!(asset.is_fungible(), "LP asset must be fungible");
        let amount = asset.value[0].as_canonical_u64();
        let current = self.supply.get()[0].as_canonical_u64();
        assert!(current >= amount, "burn exceeds LP supply");
        // LP must already be in this transaction (the burn note).
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
