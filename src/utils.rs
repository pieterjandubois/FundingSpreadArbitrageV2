use futures_util::future::BoxFuture;

use crate::DynError;

pub fn ts_hm() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub fn chunk_vec<T: Clone>(items: &[T], chunk_size: usize) -> Vec<Vec<T>> {
    if chunk_size == 0 {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut idx = 0;
    while idx < items.len() {
        let end = (idx + chunk_size).min(items.len());
        chunks.push(items[idx..end].to_vec());
        idx = end;
    }
    chunks
}

pub fn reset_backoff(backoff_ms: &mut u64) {
    *backoff_ms = 0;
}

pub async fn apply_backoff(backoff_ms: &mut u64) {
    if *backoff_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(*backoff_ms)).await;
        *backoff_ms = (*backoff_ms * 2).min(10_000);
    } else {
        *backoff_ms = 250;
    }
}

pub fn interval_secs(secs: u64) -> tokio::time::Interval {
    tokio::time::interval(std::time::Duration::from_secs(secs))
}

pub async fn subscribe_in_batches<C, T, F>(
    ctx: &mut C,
    items: &[T],
    batch_size: usize,
    delay_ms: u64,
    mut f: F,
) -> Result<(), DynError>
where
    for<'a> F: FnMut(&'a mut C, &'a [T]) -> BoxFuture<'a, Result<(), DynError>>,
{
    if batch_size == 0 {
        return Ok(());
    }

    for chunk in items.chunks(batch_size) {
        f(ctx, chunk).await?;
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    }

    Ok(())
}
