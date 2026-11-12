#![allow(clippy::useless_vec)] // `Pack` is implemented for `Vec<Bytes>`, not arrays.

use ckb_testtool::{
    builtin::ALWAYS_SUCCESS,
    ckb_error::Error as CkbError,
    ckb_types::{
        bytes::Bytes,
        core::{Cycle, TransactionBuilder, TransactionView},
        packed::{CellInput, CellOutput, OutPoint, Script},
        prelude::*,
    },
    context::Context,
};

const MAX_CYCLES: u64 = 70_000_000;
const CKB: u64 = 100_000_000;

// Error codes, mirroring `Error` in contracts/capkey-lock/src/main.rs.
const NOT_AUTHORIZED: i8 = 11;
const INVALID_GROUP_SHAPE: i8 = 12;
const OUTPUT_HAS_TYPE_SCRIPT: i8 = 14;
const EXCEEDS_PER_TX_CAP: i8 = 17;
const EXCEEDS_ALLOWANCE: i8 = 18;
const BAD_ALLOWANCE_UPDATE: i8 = 19;
const BAD_CAP_UPDATE: i8 = 20;
const BAD_SEQUENCE: i8 = 21;

#[derive(Clone, Copy)]
struct State {
    max_per_tx: u64,
    remaining: u64,
    sequence: u64,
}

impl State {
    fn encode(&self) -> Bytes {
        let mut out = Vec::with_capacity(28);
        out.extend_from_slice(b"CK01");
        out.extend_from_slice(&self.max_per_tx.to_le_bytes());
        out.extend_from_slice(&self.remaining.to_le_bytes());
        out.extend_from_slice(&self.sequence.to_le_bytes());
        Bytes::from(out)
    }

    fn after_spend(&self, spent: u64) -> State {
        State {
            max_per_tx: self.max_per_tx,
            remaining: self.remaining - spent,
            sequence: self.sequence + 1,
        }
    }
}

struct Fixture {
    context: Context,
    vault_lock: Script,
    owner_lock: Script,
    delegate_lock: Script,
    recipient_lock: Script,
}

impl Fixture {
    fn new() -> Self {
        let mut context = Context::default();
        let vault_bin = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/riscv64imac-unknown-none-elf/release/capkey-lock"
        ))
        .expect("build the contract first: make build");
        let vault_out_point = context.deploy_cell(Bytes::from(vault_bin));
        let always_success = context.deploy_cell(ALWAYS_SUCCESS.clone());

        let owner_lock = context
            .build_script(&always_success, Bytes::from_static(b"owner"))
            .unwrap();
        let delegate_lock = context
            .build_script(&always_success, Bytes::from_static(b"delegate"))
            .unwrap();
        let recipient_lock = context
            .build_script(&always_success, Bytes::from_static(b"recipient"))
            .unwrap();

        let mut args = Vec::with_capacity(64);
        args.extend_from_slice(owner_lock.calc_script_hash().as_slice());
        args.extend_from_slice(delegate_lock.calc_script_hash().as_slice());
        let vault_lock = context
            .build_script(&vault_out_point, Bytes::from(args))
            .unwrap();

        Fixture {
            context,
            vault_lock,
            owner_lock,
            delegate_lock,
            recipient_lock,
        }
    }

    fn vault_input(&mut self, capacity: u64, state: State) -> CellInput {
        let out_point = self.context.create_cell(
            CellOutput::new_builder()
                .capacity(capacity)
                .lock(self.vault_lock.clone())
                .build(),
            state.encode(),
        );
        CellInput::new_builder().previous_output(out_point).build()
    }

    fn funding_input(&mut self, lock: &Script) -> CellInput {
        let out_point: OutPoint = self.context.create_cell(
            CellOutput::new_builder()
                .capacity(100 * CKB)
                .lock(lock.clone())
                .build(),
            Bytes::new(),
        );
        CellInput::new_builder().previous_output(out_point).build()
    }

    fn cell(&self, capacity: u64, lock: &Script) -> CellOutput {
        CellOutput::new_builder()
            .capacity(capacity)
            .lock(lock.clone())
            .build()
    }

    fn verify(&mut self, tx: TransactionView) -> Result<Cycle, CkbError> {
        let tx = self.context.complete_tx(tx);
        self.context.verify_tx(&tx, MAX_CYCLES)
    }

    /// A delegate spend from a 500 CKB vault. `spent` leaves the vault in
    /// total and `paid` of it reaches the recipient; the rest is fee.
    fn delegate_spend(
        &mut self,
        vault_capacity: u64,
        before: State,
        spent: u64,
        after: State,
    ) -> Result<Cycle, CkbError> {
        let vault = self.vault_input(vault_capacity, before);
        let delegate = self.funding_input(&self.delegate_lock.clone());
        let tx = TransactionBuilder::default()
            .input(vault)
            .input(delegate)
            .output(self.cell(vault_capacity - spent, &self.vault_lock))
            .output(self.cell(spent - CKB / 100, &self.recipient_lock))
            .outputs_data(vec![after.encode(), Bytes::new()].pack())
            .build();
        self.verify(tx)
    }
}

fn start() -> State {
    State {
        max_per_tx: 100 * CKB,
        remaining: 250 * CKB,
        sequence: 0,
    }
}

fn assert_code(result: Result<Cycle, CkbError>, code: i8) {
    let err = result.expect_err("transaction should be rejected");
    let text = format!("{err:?}");
    assert!(
        text.contains(&format!("error code {code} ")),
        "expected script error code {code}, got: {text}"
    );
}

#[test]
fn delegate_spends_within_caps() {
    let mut f = Fixture::new();
    let before = start();
    let cycles = f
        .delegate_spend(500 * CKB, before, 80 * CKB, before.after_spend(80 * CKB))
        .expect("valid delegate spend");
    println!("delegate spend consumed {cycles} cycles");
}

#[test]
fn owner_can_sweep_the_vault() {
    let mut f = Fixture::new();
    let vault = f.vault_input(500 * CKB, start());
    let owner = f.funding_input(&f.owner_lock.clone());
    let tx = TransactionBuilder::default()
        .input(vault)
        .input(owner)
        .output(f.cell(599 * CKB, &f.owner_lock))
        .outputs_data(vec![Bytes::new()].pack())
        .build();
    let cycles = f.verify(tx).expect("owner sweep");
    println!("owner sweep consumed {cycles} cycles");
}

#[test]
fn rejects_spend_above_per_tx_cap() {
    let mut f = Fixture::new();
    let before = start();
    let result = f.delegate_spend(500 * CKB, before, 120 * CKB, before.after_spend(120 * CKB));
    assert_code(result, EXCEEDS_PER_TX_CAP);
}

#[test]
fn rejects_spend_above_remaining_allowance() {
    let mut f = Fixture::new();
    let before = State {
        remaining: 70 * CKB,
        ..start()
    };
    let result = f.delegate_spend(
        500 * CKB,
        before,
        80 * CKB,
        State {
            remaining: 0,
            ..before
        },
    );
    assert_code(result, EXCEEDS_ALLOWANCE);
}

#[test]
fn rapid_fire_spends_exhaust_the_allowance() {
    let mut f = Fixture::new();
    let mut state = start();
    let mut capacity = 500 * CKB;
    for _ in 0..3 {
        f.delegate_spend(capacity, state, 80 * CKB, state.after_spend(80 * CKB))
            .expect("spend within allowance");
        state = state.after_spend(80 * CKB);
        capacity -= 80 * CKB;
    }
    assert_eq!(state.remaining, 10 * CKB);
    let result = f.delegate_spend(
        capacity,
        state,
        80 * CKB,
        State {
            remaining: 0,
            ..state
        },
    );
    assert_code(result, EXCEEDS_ALLOWANCE);
}

#[test]
fn rejects_skipped_sequence() {
    let mut f = Fixture::new();
    let before = start();
    let mut after = before.after_spend(80 * CKB);
    after.sequence = 2;
    let result = f.delegate_spend(500 * CKB, before, 80 * CKB, after);
    assert_code(result, BAD_SEQUENCE);
}

#[test]
fn rejects_tampered_allowance() {
    let mut f = Fixture::new();
    let before = start();
    let mut after = before.after_spend(80 * CKB);
    after.remaining = before.remaining;
    let result = f.delegate_spend(500 * CKB, before, 80 * CKB, after);
    assert_code(result, BAD_ALLOWANCE_UPDATE);
}

#[test]
fn rejects_raised_per_tx_cap() {
    let mut f = Fixture::new();
    let before = start();
    let mut after = before.after_spend(80 * CKB);
    after.max_per_tx = 500 * CKB;
    let result = f.delegate_spend(500 * CKB, before, 80 * CKB, after);
    assert_code(result, BAD_CAP_UPDATE);
}

#[test]
fn rejects_successor_with_a_different_lock() {
    let mut f = Fixture::new();
    let before = start();
    let vault = f.vault_input(500 * CKB, before);
    let delegate = f.funding_input(&f.delegate_lock.clone());
    let tx = TransactionBuilder::default()
        .input(vault)
        .input(delegate)
        .output(f.cell(420 * CKB, &f.recipient_lock))
        .outputs_data(vec![before.after_spend(80 * CKB).encode()].pack())
        .build();
    assert_code(f.verify(tx), INVALID_GROUP_SHAPE);
}

#[test]
fn rejects_splitting_the_vault() {
    let mut f = Fixture::new();
    let before = start();
    let after = before.after_spend(80 * CKB).encode();
    let vault = f.vault_input(500 * CKB, before);
    let delegate = f.funding_input(&f.delegate_lock.clone());
    let tx = TransactionBuilder::default()
        .input(vault)
        .input(delegate)
        .output(f.cell(210 * CKB, &f.vault_lock))
        .output(f.cell(210 * CKB, &f.vault_lock))
        .outputs_data(vec![after.clone(), after].pack())
        .build();
    assert_code(f.verify(tx), INVALID_GROUP_SHAPE);
}

#[test]
fn rejects_successor_with_a_type_script() {
    let mut f = Fixture::new();
    let before = start();
    let vault = f.vault_input(500 * CKB, before);
    let delegate = f.funding_input(&f.delegate_lock.clone());
    let typed = CellOutput::new_builder()
        .capacity(420 * CKB)
        .lock(f.vault_lock.clone())
        .type_(Some(f.recipient_lock.clone()).pack())
        .build();
    let tx = TransactionBuilder::default()
        .input(vault)
        .input(delegate)
        .output(typed)
        .outputs_data(vec![before.after_spend(80 * CKB).encode()].pack())
        .build();
    assert_code(f.verify(tx), OUTPUT_HAS_TYPE_SCRIPT);
}

#[test]
fn rejects_spend_with_no_authorizing_input() {
    let mut f = Fixture::new();
    let before = start();
    let vault = f.vault_input(500 * CKB, before);
    let stranger = f.funding_input(&f.recipient_lock.clone());
    let tx = TransactionBuilder::default()
        .input(vault)
        .input(stranger)
        .output(f.cell(420 * CKB, &f.vault_lock))
        .output(f.cell(79 * CKB, &f.recipient_lock))
        .outputs_data(vec![before.after_spend(80 * CKB).encode(), Bytes::new()].pack())
        .build();
    assert_code(f.verify(tx), NOT_AUTHORIZED);
}
