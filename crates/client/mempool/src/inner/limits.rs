use std::time::{Duration, SystemTime};

use blockifier::transaction::transaction_types::TransactionType;
use mc_exec::execution::TxInfo;
use mp_chain_config::ChainConfig;

use crate::MempoolTransaction;

#[derive(Debug)]
pub struct MempoolLimits {
    pub max_transactions: usize,
    pub max_declare_transactions: usize,
    pub max_age: Duration,
}

impl MempoolLimits {
    pub fn new(chain_config: &ChainConfig) -> Self {
        Self {
            max_transactions: chain_config.mempool_tx_limit,
            max_declare_transactions: chain_config.mempool_declare_tx_limit,
            max_age: chain_config.mempool_tx_max_age,
        }
    }
    #[cfg(any(test, feature = "testing"))]
    pub fn for_testing() -> Self {
        Self {
            max_age: Duration::from_secs(10000000),
            max_declare_transactions: usize::MAX,
            max_transactions: usize::MAX,
        }
    }
}

/// Note: when a transaction is poped from the mempool by block prod, the limits will not be updated until the full
/// tick has been executed and excess transactions are added back into the mempool.
/// This means that the inner mempool may have fewer transactions than what the limits says at a given time.
#[derive(Debug)]
pub(crate) struct MempoolLimiter {
    pub config: MempoolLimits,
    current_transactions: usize,
    current_declare_transactions: usize,
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum MempoolLimitReached {
    #[error("The mempool has reached the limit of {max} transactions")]
    MaxTransactions { max: usize },
    #[error("The mempool has reached the limit of {max} declare transactions")]
    MaxDeclareTransactions { max: usize },
    #[error("The transaction age is greater than the limit of {max:?}")]
    Age { max: Duration },
}

pub(crate) struct TransactionCheckedLimits {
    check_tx_limit: bool,
    check_declare_limit: bool,
    check_age: bool,
    tx_arrived_at: SystemTime,
}

impl TransactionCheckedLimits {
    // Returns which limits apply for this transaction.
    // This struct is also used to update the limits after insertion, without having to keep a clone of the transaction around.
    // We can add more limits here as needed :)
    pub fn limits_for(tx: &MempoolTransaction) -> Self {
        match tx.tx.tx_type() {
            TransactionType::Declare => TransactionCheckedLimits {
                check_tx_limit: true,
                check_declare_limit: true,
                check_age: true,
                tx_arrived_at: tx.arrived_at,
            },
            TransactionType::DeployAccount => TransactionCheckedLimits {
                check_tx_limit: true,
                check_declare_limit: false,
                check_age: true,
                tx_arrived_at: tx.arrived_at,
            },
            TransactionType::InvokeFunction => TransactionCheckedLimits {
                check_tx_limit: true,
                check_declare_limit: false,
                check_age: true,
                tx_arrived_at: tx.arrived_at,
            },
            // L1 handler transactions are transactions added into the L1 core contract. We don't want to miss
            // any of those if possible.
            TransactionType::L1Handler => TransactionCheckedLimits {
                check_tx_limit: false,
                check_declare_limit: false,
                check_age: false,
                tx_arrived_at: tx.arrived_at,
            },
        }
    }
}

impl MempoolLimiter {
    pub fn new(limits: MempoolLimits) -> Self {
        Self { config: limits, current_transactions: 0, current_declare_transactions: 0 }
    }

    pub fn check_insert_limits(&self, to_check: &TransactionCheckedLimits) -> Result<(), MempoolLimitReached> {
        // tx limit
        if to_check.check_tx_limit && self.current_transactions >= self.config.max_transactions {
            return Err(MempoolLimitReached::MaxTransactions { max: self.config.max_transactions });
        }

        // declare tx limit
        if to_check.check_declare_limit && self.current_declare_transactions >= self.config.max_declare_transactions {
            return Err(MempoolLimitReached::MaxDeclareTransactions { max: self.config.max_declare_transactions });
        }

        // age
        if self.tx_age_exceeded(to_check) {
            return Err(MempoolLimitReached::Age { max: self.config.max_age });
        }

        Ok(())
    }

    pub fn tx_age_exceeded(&self, to_check: &TransactionCheckedLimits) -> bool {
        if to_check.check_age {
            let current_time = SystemTime::now();
            if to_check.tx_arrived_at < current_time.checked_sub(self.config.max_age).unwrap_or(SystemTime::UNIX_EPOCH)
            {
                return true;
            }
        }
        false
    }

    pub fn update_tx_limits(&mut self, limits: &TransactionCheckedLimits) {
        // We want all transactions to count toward the limit, not just those where the limit is checked.
        self.current_transactions += 1;
        if limits.check_declare_limit {
            self.current_declare_transactions += 1;
        }
    }

    pub fn mark_removed(&mut self, to_update: &TransactionCheckedLimits) {
        // These should not overflow unless block prod marks transactions as consumed even though they have not been popped.
        self.current_transactions -= 1;
        if to_update.check_declare_limit {
            self.current_declare_transactions -= 1;
        }
    }
}
