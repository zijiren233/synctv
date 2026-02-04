//! Unit of Work implementation
//!
//! Provides transactional scope for multi-repository operations.

use sqlx::{PgPool, Postgres, Transaction};
use std::ops::Deref;

use crate::Result;

/// Unit of Work for managing database transactions
///
/// Wraps a database transaction and provides access to repositories
/// that work within the same transactional context.
pub struct UnitOfWork {
    tx: Option<Transaction<'static, Postgres>>,
}

impl UnitOfWork {
    /// Begin a new transaction
    pub async fn begin(pool: &PgPool) -> Result<Self> {
        let tx = pool.begin().await?;
        Ok(Self { tx: Some(tx) })
    }

    /// Commit the transaction
    pub async fn commit(mut self) -> Result<()> {
        if let Some(tx) = self.tx.take() {
            tx.commit().await?;
        }
        Ok(())
    }

    /// Rollback the transaction
    pub async fn rollback(mut self) -> Result<()> {
        if let Some(tx) = self.tx.take() {
            tx.rollback().await?;
        }
        Ok(())
    }

    /// Get the transaction for repository operations
    pub const fn transaction(&mut self) -> &mut Transaction<'static, Postgres> {
        self.tx.as_mut().expect("Transaction already consumed")
    }
}

impl Deref for UnitOfWork {
    type Target = Transaction<'static, Postgres>;

    fn deref(&self) -> &Self::Target {
        self.tx.as_ref().expect("Transaction already consumed")
    }
}

// Implement Drop for automatic rollback on panic
impl Drop for UnitOfWork {
    fn drop(&mut self) {
        if self.tx.is_some() {
            // Transaction was not explicitly committed/rolled back
            // It will be automatically rolled back when dropped
        }
    }
}

/// Transaction wrapper for automatic commit on success
///
/// This helper allows for clean transaction handling with automatic commit/rollback.
pub async fn with_transaction<F, R>(pool: &PgPool, f: F) -> Result<R>
where
    F: for<'e> FnOnce(&mut Transaction<'e, Postgres>) -> futures::future::BoxFuture<'e, Result<R>> + Send + Sync,
    R: Send + Sync + 'static,
{
    let mut tx = pool.begin().await?;

    match f(&mut tx).await {
        Ok(result) => {
            tx.commit().await?;
            Ok(result)
        }
        Err(e) => {
            tx.rollback().await?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_transaction_commit() {
        // Integration test placeholder
    }

    #[tokio::test]
    #[ignore = "Requires database"]
    async fn test_transaction_rollback() {
        // Integration test placeholder
    }
}
