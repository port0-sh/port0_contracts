// The standard fungible faucet's token config slot, holding
// `[token_supply, max_supply, decimals, token_symbol]`.
//
// Single source: the pool and the network faucet both include this file. The
// pool writes the circulating LP supply here on mint and burn, because the
// kernel mints an asset without maintaining the config word.
fn token_config_slot() -> StorageSlotId {
    StorageSlotId::new(
        Felt::new_unchecked(10339737737024460521),
        Felt::new_unchecked(13038161168137044496),
    )
}
