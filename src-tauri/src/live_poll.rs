//! Shared throttling state machine for live provider usage polls.

use std::time::{Duration, Instant};

use crate::codex::reader::ReadFailure;

/// Minimum interval between consecutive live usage requests.
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Throttled outcome cached between live polling ticks.
#[derive(Debug)]
pub struct LiveState<T> {
    pub answer: Option<(Instant, T)>,
    pub failure: Option<ReadFailure>,
    next_request: Option<Instant>,
}

impl<T> Default for LiveState<T> {
    fn default() -> Self {
        Self {
            answer: None,
            failure: None,
            next_request: None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum LiveStep<T> {
    Answer(T),
    Failure(ReadFailure),
    Request,
}

impl<T: Clone> LiveState<T> {
    pub fn step(&self, now: Instant) -> LiveStep<T> {
        if self.next_request.is_none_or(|at| now >= at) {
            return LiveStep::Request;
        }
        match (&self.failure, &self.answer) {
            (Some(failure), _) => LiveStep::Failure(failure.clone()),
            (None, Some((_, snapshot))) => LiveStep::Answer(snapshot.clone()),
            (None, None) => LiveStep::Request,
        }
    }

    pub fn record(&mut self, now: Instant, result: &Result<T, ReadFailure>) {
        let wait = match result {
            Ok(snapshot) => {
                self.answer = Some((now, snapshot.clone()));
                self.failure = None;
                MIN_INTERVAL
            }
            Err(failure) => {
                self.failure = Some(failure.clone());
                failure
                    .retry_after_seconds
                    .map_or(MIN_INTERVAL, Duration::from_secs)
                    .max(MIN_INTERVAL)
            }
        };
        self.next_request = Some(now + wait);
    }
}
