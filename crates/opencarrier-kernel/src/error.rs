//! Kernel-specific error types.

use opencarrier_types::error::OpenCarrierError;
use thiserror::Error;

/// Kernel error type wrapping OpenCarrierError with kernel-specific context.
#[derive(Error, Debug)]
pub enum KernelError {
    /// A wrapped OpenCarrierError.
    #[error(transparent)]
    OpenCarrier(#[from] OpenCarrierError),

    /// The kernel failed to boot.
    #[error("Boot failed: {0}")]
    BootFailed(String),
}

/// Alias for kernel results.
pub type KernelResult<T> = Result<T, KernelError>;
