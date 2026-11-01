//! Timers for nudges (`docs/product-spec.md` 10.2).

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
}
