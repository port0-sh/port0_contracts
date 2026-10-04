use miden::{Asset, Felt, Word};

use crate::error::ConversionError;

pub fn compose_asset(asset_key: Word, amount: u64) -> Result<Asset, ConversionError> {
    Ok(Asset::new(
        asset_key,
        Word::from([
            Felt::new(amount).map_err(|_| ConversionError::U64ToFelt)?,
            Felt::ZERO,
            Felt::ZERO,
            Felt::ZERO,
        ]),
    ))
}
