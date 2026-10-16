use std::time::{Duration, Instant};

const QUIET_PERIOD: Duration = Duration::from_millis(2);
const MAX_DELAY: Duration = Duration::from_millis(5);

#[derive(Default)]
pub(super) struct Redraw {
    pending: Option<Pending>,
}

struct Pending {
    first: Instant,
    deadline: Instant,
}

impl Redraw {
    pub(super) fn defer(&mut self, now: Instant) {
        let pending = self.pending.get_or_insert(Pending {
            first: now,
            deadline: now + QUIET_PERIOD,
        });
        pending.deadline = (now + QUIET_PERIOD).min(pending.first + MAX_DELAY);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.deadline)
    }

    pub(super) fn presented(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_repaint_waits_for_quiet_and_then_resets() {
        let start = Instant::now();
        let mut redraw = Redraw::default();
        assert_eq!(redraw.deadline(), None);
        for millis in [0, 1, 2] {
            let now = start + Duration::from_millis(millis);
            redraw.defer(now);
            assert_eq!(redraw.deadline(), Some(now + QUIET_PERIOD));
        }
        assert_eq!(redraw.deadline(), Some(start + Duration::from_millis(4)));
        redraw.presented();
        assert_eq!(redraw.deadline(), None);
        let later = start + Duration::from_secs(1);
        redraw.defer(later);
        assert_eq!(redraw.deadline(), Some(later + QUIET_PERIOD));
    }

    #[test]
    fn continuous_output_cannot_postpone_the_deadline() {
        let start = Instant::now();
        let mut redraw = Redraw::default();
        for millis in 0..20 {
            let now = start + Duration::from_millis(millis);
            redraw.defer(now);
            assert!(redraw.deadline().unwrap() <= start + MAX_DELAY);
        }
        assert_eq!(redraw.deadline(), Some(start + MAX_DELAY));
    }
}
