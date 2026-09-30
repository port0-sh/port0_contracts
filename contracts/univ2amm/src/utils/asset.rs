use miden::{AccountId, Asset, Felt, Word};

use crate::error::ConversionError;

pub fn get_asset_key_from_faucet_id(account_id: &AccountId) -> Result<Word, ConversionError> {
    let suffix = Felt::new(account_id.suffix.as_canonical_u64() | 1)
        .map_err(|_| ConversionError::AssetKeyToAccountId)?;
    Ok(Word::from([
        Felt::ZERO,
        Felt::ZERO,
        suffix,
        account_id.prefix,
    ]))
}

pub fn get_account_id_from_asset_key(asset_key: &Word) -> Result<AccountId, ConversionError> {
    let suffix = Felt::new(asset_key[2].as_canonical_u64() & !0xff)
        .map_err(|_| ConversionError::AccountIdToAssetKey)?;
    Ok(AccountId::new(asset_key[3], suffix))
}

pub fn compose_asset(faucet_id: AccountId, amount: u64) -> Result<Asset, ConversionError> {
    Ok(Asset::new(
        get_asset_key_from_faucet_id(&faucet_id)?,
        Word::from([
            Felt::new(amount).map_err(|_| ConversionError::U64ToFelt)?,
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]),
    ))
}
