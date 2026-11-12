//! capkey-lock: a vault lock with a spend-capped delegate path.
//!
//! Two ways to unlock a vault Cell:
//!
//! * Owner path: the transaction contains an input locked by `owner_lock_hash`.
//!   Nothing else is checked.
//! * Delegate path: the transaction contains an input locked by
//!   `delegate_lock_hash` (the delegate's own lock verifies the signature over
//!   the whole transaction) and the vault transition obeys the CK01 rules.

#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

use ckb_std::{
    ckb_constants::Source,
    ckb_types::{bytes::Bytes, prelude::*},
    error::SysError,
    high_level::{
        load_cell_capacity, load_cell_data, load_cell_lock_hash, load_cell_type_hash, load_script,
        load_script_hash, QueryIter,
    },
};

const ARGS_LEN: usize = 64;
const DATA_LEN: usize = 28;
const MAGIC: [u8; 4] = *b"CK01";

#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    IndexOutOfBound = 1,
    ItemMissing,
    LengthNotEnough,
    Encoding,
    InvalidArgs = 10,
    NotAuthorized,
    InvalidGroupShape,
    OutputLockChanged,
    OutputHasTypeScript,
    InvalidData,
    CapacityNotReduced,
    ExceedsPerTxCap,
    ExceedsAllowance,
    BadAllowanceUpdate,
    BadCapUpdate,
    BadSequence,
}

impl From<SysError> for Error {
    fn from(err: SysError) -> Self {
        match err {
            SysError::IndexOutOfBound => Error::IndexOutOfBound,
            SysError::ItemMissing => Error::ItemMissing,
            SysError::LengthNotEnough(_) => Error::LengthNotEnough,
            SysError::Encoding => Error::Encoding,
            _ => Error::Encoding,
        }
    }
}

struct VaultArgs {
    owner_lock_hash: [u8; 32],
    delegate_lock_hash: [u8; 32],
}

struct VaultData {
    max_per_tx: u64,
    remaining_allowance: u64,
    last_sequence: u64,
}

fn parse_args(args: &[u8]) -> Result<VaultArgs, Error> {
    if args.len() != ARGS_LEN {
        return Err(Error::InvalidArgs);
    }
    let mut owner_lock_hash = [0u8; 32];
    let mut delegate_lock_hash = [0u8; 32];
    owner_lock_hash.copy_from_slice(&args[..32]);
    delegate_lock_hash.copy_from_slice(&args[32..]);
    Ok(VaultArgs {
        owner_lock_hash,
        delegate_lock_hash,
    })
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn parse_data(data: &[u8]) -> Result<VaultData, Error> {
    if data.len() != DATA_LEN || data[..4] != MAGIC {
        return Err(Error::InvalidData);
    }
    Ok(VaultData {
        max_per_tx: read_u64(data, 4),
        remaining_allowance: read_u64(data, 12),
        last_sequence: read_u64(data, 20),
    })
}

fn input_with_lock_exists(lock_hash: &[u8; 32]) -> bool {
    QueryIter::new(load_cell_lock_hash, Source::Input).any(|hash| &hash == lock_hash)
}

fn run() -> Result<(), Error> {
    let script = load_script()?;
    let raw_args: Bytes = script.args().unpack();
    let args = parse_args(&raw_args)?;

    if input_with_lock_exists(&args.owner_lock_hash) {
        return Ok(());
    }
    if !input_with_lock_exists(&args.delegate_lock_hash) {
        return Err(Error::NotAuthorized);
    }
    check_delegate_spend()
}

fn check_delegate_spend() -> Result<(), Error> {
    let own_hash = load_script_hash()?;

    // Exactly one vault Cell is consumed and exactly one is created, so a
    // single transaction cannot split or merge vaults to dodge the caps.
    if QueryIter::new(load_cell_capacity, Source::GroupInput).count() != 1
        || QueryIter::new(load_cell_lock_hash, Source::Output)
            .filter(|hash| hash == &own_hash)
            .count()
            != 1
    {
        return Err(Error::InvalidGroupShape);
    }

    // The successor must sit at output 0 with the same script.
    if load_cell_lock_hash(0, Source::Output)? != own_hash {
        return Err(Error::OutputLockChanged);
    }
    if load_cell_type_hash(0, Source::Output)?.is_some() {
        return Err(Error::OutputHasTypeScript);
    }

    let input = parse_data(&load_cell_data(0, Source::GroupInput)?)?;
    let output = parse_data(&load_cell_data(0, Source::Output)?)?;

    let in_capacity = load_cell_capacity(0, Source::GroupInput)?;
    let out_capacity = load_cell_capacity(0, Source::Output)?;
    let spent = match in_capacity.checked_sub(out_capacity) {
        Some(spent) if spent > 0 => spent,
        _ => return Err(Error::CapacityNotReduced),
    };

    if spent > input.max_per_tx {
        return Err(Error::ExceedsPerTxCap);
    }
    if spent > input.remaining_allowance {
        return Err(Error::ExceedsAllowance);
    }
    if output.max_per_tx != input.max_per_tx {
        return Err(Error::BadCapUpdate);
    }
    if output.remaining_allowance != input.remaining_allowance - spent {
        return Err(Error::BadAllowanceUpdate);
    }
    if input.last_sequence.checked_add(1) != Some(output.last_sequence) {
        return Err(Error::BadSequence);
    }
    Ok(())
}

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(err) => err as i8,
    }
}
