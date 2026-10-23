//! Network faucet: a fixed-supply token whose faucet is a network account.
//!
//! A regular faucet is a keyed account, and on a network that gates keyed
//! accounts its first transaction needs an invitation code. A network account is
//! exempt, so building the faucet as one removes that requirement: launching a
//! token costs its creator nothing but the note fees.
//!
//! The account carries three components: this one, which owns the launch and the
//! supply accounting; the standard basic fungible faucet, which supplies the
//! name, symbol, and decimals a wallet reads and is what makes the account
//! recognizable as a faucet; and the network auth, which allowlists the launch
//! note and prices it.
//!
//! A launch is one-way and total: the owner's note mints the whole maximum supply
//! once, pays it to a recipient, and records the supply so the faucet reports it.
//! The standard component's own minting procedure stays unreachable, because this
//! account installs no mint policy and that procedure aborts without one.

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

use miden::*;

/// The account's own asset id, which is the faucet id its token is denominated
/// in. A network account mints assets whose issuer is itself, and this is the id
/// word for that issuer: `[0, 0, suffix | 0x11, prefix]`, where `0x11` is the
/// fungible asset id metadata byte.
fn own_asset_key() -> Word {
    let id = active_account::get_id();
    Word::new([
        Felt::ZERO,
        Felt::ZERO,
        Felt::new_unchecked(id.suffix.as_canonical_u64() | 0x11),
        id.prefix,
    ])
}

/// Script root of the standard P2ID note, which carries the supply out.
fn p2id_root() -> Word {
    Word::new([
        Felt::new_unchecked(2560749703961673979),
        Felt::new_unchecked(3134968106512891729),
        Felt::new_unchecked(4508201388898539264),
        Felt::new_unchecked(15729441787511219738),
    ])
}

/// The standard faucet component's token config slot, holding
/// `[token_supply, max_supply, decimals, token_symbol]`.
fn token_config_slot() -> StorageSlotId {
    StorageSlotId::new(
        Felt::new_unchecked(10339737737024460521),
        Felt::new_unchecked(13038161168137044496),
    )
}

#[component_storage]
struct NetworkFaucetStorage {
    #[storage(description = "Account allowed to launch the token")]
    owner: StorageValue<AccountId>,
    #[storage(description = "1 once the supply has been minted")]
    launched: StorageValue<Felt>,
}

#[component]
trait NetworkFaucet {
    /// Mints the whole maximum supply to `recipient`, once.
    ///
    /// The launch note passes `recipient`, an output-note `serial`, and the
    /// note's tag. The supply comes from the faucet's own token config, so the
    /// amount minted is exactly the maximum the metadata advertises.
    #[account_procedure]
    fn launch(&mut self, recipient: AccountId, serial_num: Word, tag: Felt);
}

#[component]
impl NetworkFaucet for NetworkFaucetStorage {
    fn launch(&mut self, recipient: AccountId, serial_num: Word, tag: Felt) {
        assert!(
            active_note::get_sender().eq(&self.owner.get()),
            "Only the faucet owner may launch the token"
        );
        assert!(
            self.launched.get().as_canonical_u64() == 0,
            "The token has already been launched"
        );

        let config = storage::get_item(token_config_slot());
        let max_supply = config[1].as_canonical_u64();
        assert!(max_supply > 0, "The token has no maximum supply");

        self.launched.set(Felt::ONE);

        // The kernel mints an asset whose issuer is the executing account, and
        // does not maintain the config word, so the supply is recorded here.
        let asset = Asset::new(
            own_asset_key(),
            Word::new([
                Felt::new_unchecked(max_supply),
                Felt::ZERO,
                Felt::ZERO,
                Felt::ZERO,
            ]),
        );
        faucet::mint(asset);
        storage::set_item(
            token_config_slot(),
            Word::new([Felt::new_unchecked(max_supply), config[1], config[2], config[3]]),
        );

        let note_recipient = note::build_recipient(
            serial_num,
            p2id_root(),
            alloc::vec![recipient.suffix, recipient.prefix, Felt::ZERO, Felt::ZERO],
        );
        let note_idx = output_note::create(
            Tag::from(tag),
            NoteType::from(Felt::new_unchecked(1)),
            note_recipient,
        );
        // The minted asset goes into the output note directly; it never enters
        // the vault, which is why there is no `remove_asset` here. Pools mint LP
        // the same way.
        output_note::add_asset(asset, note_idx);
    }
}
