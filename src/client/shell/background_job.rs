//! orcha: work that must not block the input loop. A job runs one function
//! on its own named thread; the shell polls for the result on its ticks.

use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The job's thread ended without a result (it panicked).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct JobLost;

impl std::fmt::Display for JobLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("it stopped unexpectedly")
    }
}

pub(super) struct BackgroundJob<T> {
    result: Receiver<T>,
}

impl<T: Send + 'static> BackgroundJob<T> {
    /// Runs `work` on a thread named `name`. None (logged) when no thread
    /// could start.
    pub(super) fn spawn(name: &str, work: impl FnOnce() -> T + Send + 'static) -> Option<Self> {
        let (tx, result) = mpsc::channel();
        match crate::thread_spawn::spawn_named(name, move || {
            let _ = tx.send(work());
        }) {
            Ok(_) => Some(Self { result }),
            Err(err) => {
                tracing::warn!(err = %err, name, "orcha: a background job did not start");
                None
            }
        }
    }
}

impl<T> BackgroundJob<T> {
    /// The result once it is in; `JobLost` when the thread ended without
    /// one; None while it runs.
    pub(super) fn poll(&self) -> Option<Result<T, JobLost>> {
        match self.result.try_recv() {
            Ok(value) => Some(Ok(value)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JobLost)),
        }
    }

    /// Polls the job in `slot`, emptying the slot once it finished.
    pub(super) fn take_finished(slot: &mut Option<Self>) -> Option<Result<T, JobLost>> {
        let result = slot.as_ref()?.poll()?;
        *slot = None;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait<T>(slot: &mut Option<BackgroundJob<T>>) -> Result<T, JobLost> {
        for _ in 0..2_000 {
            if let Some(result) = BackgroundJob::take_finished(slot) {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the job never finished");
    }

    #[test]
    fn a_job_hands_back_its_result_once_and_empties_its_slot() {
        let mut slot = BackgroundJob::spawn("orcha-test-job", || 6 * 7);
        assert_eq!(wait(&mut slot), Ok(42));
        assert!(slot.is_none());
        assert!(BackgroundJob::take_finished(&mut slot).is_none());
    }

    #[test]
    fn a_job_whose_thread_dies_is_reported_lost() {
        let mut slot = BackgroundJob::<u8>::spawn("orcha-test-job-lost", || {
            std::panic::resume_unwind(Box::new("boom"))
        });
        assert_eq!(wait(&mut slot), Err(JobLost));
        assert!(slot.is_none());
    }
}
