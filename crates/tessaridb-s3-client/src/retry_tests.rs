use std::cell::Cell;
use std::time::Duration;

use tokio::time::Instant;

use super::RetryPolicy;
use crate::Error;

fn slow_down() -> Error {
    Error::Service {
        status: 503,
        code: Some("SlowDown".to_owned()),
        message: None,
        request_id: None,
    }
}

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(200),
        max_delay: Duration::from_secs(5),
        max_elapsed: Duration::from_secs(120),
    }
}

#[tokio::test(start_paused = true)]
async fn transient_failures_are_retried_until_one_succeeds() {
    let calls = Cell::new(0_u32);
    let result = policy()
        .run(|attempt| {
            calls.set(attempt);
            async move {
                if attempt < 3 {
                    Err(slow_down())
                } else {
                    Ok("done")
                }
            }
        })
        .await;
    assert_eq!(result.ok(), Some("done"));
    assert_eq!(calls.get(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_permanent_failure_is_returned_at_once_unchanged() {
    let calls = Cell::new(0_u32);
    let started = Instant::now();
    let result: crate::Result<()> = policy()
        .run(|attempt| {
            calls.set(attempt);
            async {
                Err(Error::Service {
                    status: 404,
                    code: Some("NoSuchKey".to_owned()),
                    message: None,
                    request_id: None,
                })
            }
        })
        .await;
    assert_eq!(calls.get(), 1);
    assert!(matches!(result, Err(Error::Service { status: 404, .. })));
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn exhaustion_reports_the_attempts_and_the_last_cause_within_the_backoff_ceilings() {
    let started = Instant::now();
    let result: crate::Result<()> = policy().run(|_| async { Err(slow_down()) }).await;
    match result {
        Err(Error::RetriesExhausted { attempts, last }) => {
            assert_eq!(attempts, 3);
            assert_eq!(last.s3_code(), Some("SlowDown"));
        }
        other => panic!("expected exhaustion, got {other:?}"),
    }
    // Two pauses, each at most its ceiling: 200 ms then 400 ms.
    assert!(
        started.elapsed() <= Duration::from_millis(600),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn no_attempt_starts_past_the_time_budget() {
    let calls = Cell::new(0_u32);
    let tight = RetryPolicy {
        max_elapsed: Duration::ZERO,
        ..policy()
    };
    let result: crate::Result<()> = tight
        .run(|attempt| {
            calls.set(attempt);
            async { Err(Error::Timeout(Duration::from_secs(1))) }
        })
        .await;
    assert_eq!(calls.get(), 1);
    assert!(matches!(
        result,
        Err(Error::RetriesExhausted { attempts: 1, .. })
    ));
}

#[test]
fn each_pause_stays_under_its_ceiling_and_the_ceiling_is_capped() {
    let policy = policy();
    for _ in 0..200 {
        assert!(policy.pause(1) <= Duration::from_millis(200));
        assert!(policy.pause(2) <= Duration::from_millis(400));
        assert!(policy.pause(40) <= Duration::from_secs(5));
    }
}
