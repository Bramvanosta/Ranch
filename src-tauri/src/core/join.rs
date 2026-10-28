//! Await several futures at once, results in order. A connection
//! actor runs reads concurrently, so a command asking it for N
//! things sends them together instead of one after the other. (No `futures`
//! crate in the tree for a `join_all`.)

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

pub async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
    let mut pending: Vec<Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut waiting = false;
        for (slot, future) in out.iter_mut().zip(pending.iter_mut()) {
            if slot.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => *slot = Some(value),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    out.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn runs_them_together_in_order() {
        let started = Instant::now();
        let out = join_all(
            [30u64, 10, 20]
                .into_iter()
                .map(|ms| async move {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    ms
                })
                .collect(),
        )
        .await;
        assert_eq!(out, vec![30, 10, 20]);
        assert!(
            started.elapsed() < Duration::from_millis(55),
            "not one after the other"
        );
    }
}
