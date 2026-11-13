# CapKey

A CKB lock script that lets a vault Cell be spent by a delegate key under hard on-chain limits. The owner can always take the funds back.

Written in Rust with `ckb-std` and tested in the real CKB-VM with `ckb-testtool`.

## What the lock enforces

A vault Cell can be unlocked in two ways.

**Owner path.** The transaction spends an input locked by `owner_lock_hash`. The script accepts it without further checks, so the owner can sweep, top up or rotate the delegate at any time.

**Delegate path.** The transaction spends an input locked by `delegate_lock_hash`. That lock verifies the signature over the whole transaction, so CapKey does no cryptography of its own. Any CKB lock can act as the delegate, including secp256k1 or a WebAuthn lock such as JoyID. On top of that the script requires:

1. One vault Cell in and exactly one out, so a spend cannot split or merge vaults.
2. The successor sits at output 0 with the same lock and no type script.
3. `spent = input.capacity - output[0].capacity` is greater than zero. It covers the recipient payment and the fee.
4. `spent <= max_per_tx`.
5. `spent <= remaining_allowance`.
6. The successor keeps `max_per_tx`, lowers `remaining_allowance` by exactly `spent` and raises `last_sequence` by one.

Because the allowance lives in Cell data, a compromised delegate cannot get around the cap by sending many transactions. Each one has to consume the previous successor.

## Layout

Lock args, 64 bytes:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 32 | `owner_lock_hash` |
| 32 | 32 | `delegate_lock_hash` |

Cell data, 28 bytes, little endian:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic `CK01` |
| 4 | 8 | `max_per_tx` in shannons |
| 12 | 8 | `remaining_allowance` in shannons |
| 20 | 8 | `last_sequence` |

## Measured

| Item | Value |
| --- | --- |
| Binary size | 27,712 bytes |
| Delegate spend | 38,085 cycles |
| Owner sweep | 22,648 cycles |

The delegate figure includes the delegate's always-success lock in the test harness. A real signature check adds the cost of that lock.

## Build and test

You need Rust stable and the RISC-V target:

```bash
rustup target add riscv64imac-unknown-none-elf
make test
```

The suite has 12 tests. Each rejection test asserts the exact script error code, so a test cannot pass because of an unrelated failure.

| Test | Expected result |
| --- | --- |
| Delegate spend within caps | accepted |
| Owner sweep | accepted |
| Spend above `max_per_tx` | `ExceedsPerTxCap` |
| Spend above remaining allowance | `ExceedsAllowance` |
| Three spends then a fourth over the allowance | fourth gives `ExceedsAllowance` |
| Skipped sequence number | `BadSequence` |
| Allowance not lowered by `spent` | `BadAllowanceUpdate` |
| Raised `max_per_tx` in the successor | `BadCapUpdate` |
| Successor moved to another lock | `InvalidGroupShape` |
| Vault split into two outputs | `InvalidGroupShape` |
| Successor carries a type script | `OutputHasTypeScript` |
| No owner or delegate input | `NotAuthorized` |

## Limits

- CKB has no upper bound on when a transaction can be committed, so a script cannot enforce "valid until time T". CapKey does not claim expiry. Limit exposure with the allowance and revoke by sweeping.
- The caps apply to the vault Cell's capacity only. xUDT and Spore assets are out of scope.
- The recipient is not restricted. The delegate can pay any address up to the caps.
- This is testnet-grade code and has not been audited.

## License

MIT
