//! Timers for nudges (`docs/product-spec.md` 10.2) and the no-response notice (4.6).

/// Fires `interval` after the condition starts holding, then every `interval`. Resets when it ends.
#[derive(Debug, Clone, Default)]
pub struct NudgeTimer {
    due_at: Option<i64>,
}

impl NudgeTimer {
    pub fn tick(&mut self, condition: bool, now: i64, interval: i64) -> bool {
        if !condition {
            self.due_at = None;
            return false;
        }
        match self.due_at {
            None => {
                self.due_at = Some(now + interval);
                false
            }
            Some(due) if now >= due => {
                self.due_at = Some(now + interval);
                true
            }
            Some(_) => false,
        }
    }
}

pub const NO_RESPONSE_AFTER_MS: i64 = 60_000;

/// Set when a response is expected: a session started or the user posted. A tool call clears it.
#[derive(Debug, Clone, Default)]
pub struct ResponseWatch {
    waiting_since: Option<i64>,
}

impl ResponseWatch {
    pub fn expect(&mut self, now: i64) {
        self.waiting_since.get_or_insert(now);
    }

    pub fn tool_called(&mut self) {
        self.waiting_since = None;
    }

    pub fn silent(&self, now: i64) -> bool {
        self.waiting_since
            .is_some_and(|since| now - since >= NO_RESPONSE_AFTER_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudges_after_the_interval_then_repeat_and_reset() {
        let mut timer = NudgeTimer::default();
        assert!(!timer.tick(true, 0, 300));
        assert!(!timer.tick(true, 299, 300));
        assert!(timer.tick(true, 300, 300));
        assert!(!timer.tick(true, 301, 300));
        assert!(timer.tick(true, 600, 300));
        assert!(!timer.tick(false, 700, 300));
        assert!(!timer.tick(true, 800, 300));
        assert!(!timer.tick(true, 1099, 300));
        assert!(timer.tick(true, 1100, 300));
    }

    #[test]
    fn silence_counts_from_the_first_expectation_until_a_tool_call() {
        let mut watch = ResponseWatch::default();
        assert!(!watch.silent(1_000_000));
        watch.expect(0);
        watch.expect(50_000);
        assert!(!watch.silent(59_999));
        assert!(watch.silent(60_000));
        watch.tool_called();
        assert!(!watch.silent(120_000));
    }
}
