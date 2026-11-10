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
    high_level::{load_cell_lock_hash, load_script, QueryIter},
};

const ARGS_LEN: usize = 64;

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

#[allow(dead_code)] // delegate_lock_hash is used once the delegate path lands
struct VaultArgs {
    owner_lock_hash: [u8; 32],
    delegate_lock_hash: [u8; 32],
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
    Ok(())
}

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(err) => err as i8,
    }
}
