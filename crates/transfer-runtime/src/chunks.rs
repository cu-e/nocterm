//! Accounting for the chunk batches of pipelined streams.
use nocterm_transfers::{StreamBudget, TransferCause};

/// Total bytes of a batch.
pub(crate) fn total(chunks: &[Vec<u8>]) -> usize {
    chunks.iter().map(Vec::len).sum()
}
/// Checks every chunk of a freshly read batch against the budget, counting the
/// earlier chunks of the same batch as buffered.
pub(crate) fn accept_batch(budget: &StreamBudget, chunks: &[Vec<u8>]) -> Result<(), TransferCause> {
    let mut buffered = 0usize;
    for chunk in chunks {
        budget.accept_buffered(buffered, chunk.len())?;
        buffered += chunk.len();
    }
    Ok(())
}
