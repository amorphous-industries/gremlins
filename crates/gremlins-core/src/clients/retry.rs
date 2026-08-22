use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

pub async fn with_retry<F, Fut, T, E>(
    backoff: &[f64],
    classify: impl Fn(&E) -> bool,
    mut on_retry: impl FnMut(usize, &E, f64),
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let max_attempts = backoff.len() + 1;
    for attempt in 0..max_attempts {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if attempt == backoff.len() || !classify(&e) {
                    return Err(e);
                }
                let wait = backoff[attempt];
                on_retry(attempt, &e, wait);
                sleep(Duration::from_secs_f64(wait)).await;
            }
        }
    }
    unreachable!()
}
