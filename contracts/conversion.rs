//! Asset conversion contract for Stellar assets.

use soroban_sdk::{contract, contractimpl, contracterror, panic_with_error, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ConversionError {
    SameToken = 1,
    InvalidAmount = 2,
    RateNotFound = 3,
    // [SEC-CONV-01] Explicit overflow error instead of a generic &'static str.
    Overflow = 4,
    // [SEC-CONV-02] Division-by-zero guard.
    DivisionByZero = 5,
    // [SEC-CONV-03] Zero conversion result guard.
    ZeroResult = 6,
}

pub struct MockPriceOracle;

impl MockPriceOracle {
    /// Returns `(numerator, denominator)` representing the exchange rate
    /// `from → to`, i.e. `converted = amount * num / denom`.
    ///
    /// # Security
    /// - [SEC-CONV-02] Callers must validate that `denom != 0` before use.
    /// - Real implementations must replace this with an authenticated on-chain
    ///   oracle; unauthenticated off-chain price feeds are a manipulation vector.
    pub fn get_rate(_from: &Address, _to: &Address) -> Option<(u32, u32)> {
        // Mock: 1 from_token = 2 to_token
        Some((2, 1))
    }
}

#[contract]
pub struct ConversionContract;

#[contractimpl]
impl ConversionContract {
    /// Converts `amount` of `from_token` to `to_token` using the oracle rate.
    ///
    /// Returns the converted amount (truncated, not rounded).
    ///
    /// # Security
    /// - [SEC-CONV-04] `user.require_auth()` ensures only the token owner can
    ///   initiate a conversion; previously the caller was unchecked.
    /// - [SEC-CONV-01] Overflow on `amount * numerator` surfaces as a typed
    ///   `Overflow` error rather than a panic string.
    /// - [SEC-CONV-02] A zero denominator from the oracle triggers
    ///   `DivisionByZero` before the division is attempted.
    /// - [SEC-CONV-03] A zero conversion result is rejected; it indicates an
    ///   amount too small for the current rate, preventing dust-drain attacks.
    /// - Same-token conversion and non-positive amounts are rejected up front.
    pub fn convert_assets(
        env: Env,
        user: Address,
        from_token: Address,
        to_token: Address,
        amount: i128,
    ) -> Result<i128, ConversionError> {
        // [SEC-CONV-04] Authenticate the initiating user.
        user.require_auth();

        if from_token == to_token {
            return Err(ConversionError::SameToken);
        }
        if amount <= 0 {
            return Err(ConversionError::InvalidAmount);
        }

        let (num, denom) =
            MockPriceOracle::get_rate(&from_token, &to_token).ok_or(ConversionError::RateNotFound)?;

        // [SEC-CONV-02] Guard against a zero denominator from the oracle.
        if denom == 0 {
            return Err(ConversionError::DivisionByZero);
        }

        // [SEC-CONV-01] Checked multiplication before dividing.
        let numerator_product = amount
            .checked_mul(num as i128)
            .ok_or(ConversionError::Overflow)?;

        let converted = numerator_product
            .checked_div(denom as i128)
            .ok_or(ConversionError::DivisionByZero)?;

        // [SEC-CONV-03] Reject dust conversions that round to zero.
        if converted == 0 {
            return Err(ConversionError::ZeroResult);
        }

        env.events().publish(
            (
                soroban_sdk::symbol_short!("convert"),
                user.clone(),
            ),
            (
                from_token.clone(),
                to_token.clone(),
                amount,
                converted,
                env.ledger().timestamp(),
            ),
        );

        Ok(converted)
    }
}