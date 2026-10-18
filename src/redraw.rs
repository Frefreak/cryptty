use std::time::{Duration, Instant};

const QUIET_PERIOD: Duration = Duration::from_millis(2);
const INPUT_PRIORITY: Duration = Duration::from_millis(50);
const MAX_DELAY: Duration = Duration::from_millis(5);

#[derive(Default)]
pub(super) struct Redraw {
    pending: Option<Pending>,
    next_frame: Option<Instant>,
    input_until: Option<Instant>,
}

struct Pending {
    first: Instant,
    deadline: Instant,
}

impl Redraw {
    pub(super) fn input(&mut self, now: Instant) {
        self.input_until = Some(now + INPUT_PRIORITY);
    }

    pub(super) fn cancel_input(&mut self) {
        self.input_until = None;
    }

    pub(super) fn input_active(&self, now: Instant) -> bool {
        self.input_until.is_some_and(|until| now < until)
    }

    pub(super) fn defer(&mut self, now: Instant) {
        let pending = self.pending.get_or_insert(Pending {
            first: now,
            deadline: now + QUIET_PERIOD,
        });
        pending.deadline = (now + QUIET_PERIOD).min(pending.first + MAX_DELAY);
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| {
            self.next_frame
                .map_or(p.deadline, |next| p.deadline.max(next))
        })
    }

    pub(super) fn presented(&mut self, now: Instant, interval: Duration) {
        self.pending = None;
        self.next_frame = now.checked_add(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_priority_covers_split_output_and_expires() {
        let start = Instant::now();
        let mut redraw = Redraw::default();
        assert!(!redraw.input_active(start));
        redraw.input(start);
        redraw.presented(start, Duration::ZERO);
        redraw.defer(start + Duration::from_millis(1));
        let due = redraw.deadline().unwrap();
        assert!(redraw.input_active(due));
        redraw.presented(due, Duration::ZERO);
        assert!(redraw.input_active(start + INPUT_PRIORITY / 2));
        assert!(!redraw.input_active(start + INPUT_PRIORITY));
        redraw.defer(start + INPUT_PRIORITY);
        assert!(!redraw.input_active(redraw.deadline().unwrap()));
    }

    #[test]
    fn repeated_input_extends_priority_and_focus_loss_cancels_it() {
        let start = Instant::now();
        let mut redraw = Redraw::default();
        redraw.input(start);
        let later = start + INPUT_PRIORITY / 2;
        redraw.input(later);
        assert!(redraw.input_active(start + INPUT_PRIORITY));
        assert!(!redraw.input_active(later + INPUT_PRIORITY));
        redraw.cancel_input();
        assert!(!redraw.input_active(later));
    }

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
        redraw.presented(start, Duration::ZERO);
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

    #[test]
    fn frame_pacing_preserves_pending_output_until_the_next_frame() {
        let start = Instant::now();
        let interval = Duration::from_millis(16);
        let mut redraw = Redraw::default();
        redraw.presented(start, interval);
        assert_eq!(redraw.deadline(), None);
        for millis in 1..16 {
            redraw.defer(start + Duration::from_millis(millis));
            assert_eq!(redraw.deadline(), Some(start + interval));
        }
        redraw.presented(start + interval, interval);
        assert_eq!(redraw.deadline(), None);
        redraw.defer(start + Duration::from_millis(40));
        assert_eq!(redraw.deadline(), Some(start + Duration::from_millis(42)));
    }
}
