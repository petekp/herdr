use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crossterm::event::MouseEventKind;

/// Gap at or above which a wheel event is a discrete notch, a momentum tail,
/// or a slow deliberate scroll rather than part of a dense trackpad stream.
/// Nothing drifts at these rates, so such events bypass the axis lock.
pub(super) const WHEEL_NOTCH_GAP: Duration = Duration::from_millis(40);
/// Quiet time after which the next wheel event starts a new gesture on its
/// own axis, whatever the previous gesture was doing.
pub(super) const WHEEL_QUIET_GAP: Duration = Duration::from_millis(200);
/// The other axis takes over once it has all but one of this many most
/// recent events. A stray or two never manages that; a real gesture on the
/// other axis does within a few events, even while the old gesture's momentum
/// is still arriving in between.
const TAKEOVER_RUN: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WheelAxis {
    Vertical,
    Horizontal,
}

impl WheelAxis {
    pub(super) fn of(kind: MouseEventKind) -> Option<Self> {
        match kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => Some(Self::Vertical),
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => Some(Self::Horizontal),
            _ => None,
        }
    }
}

/// The axis a dense wheel stream currently belongs to, the way touch
/// platforms lock a scroll gesture to one direction. Fingers drift, so
/// without the lock a vertical scroll would nudge the tab swipe and a swipe
/// would scroll the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WheelAxisLock {
    axis: WheelAxis,
    last_event: Instant,
    /// Axes of the last `TAKEOVER_RUN` dense events, oldest first.
    recent: VecDeque<WheelAxis>,
}

impl WheelAxisLock {
    /// Whether a wheel event on `axis` should be delivered.
    pub(super) fn admit(lock: &mut Option<Self>, axis: WheelAxis, now: Instant) -> bool {
        let Some(current) = lock else {
            *lock = Some(Self {
                axis,
                last_event: now,
                recent: VecDeque::from([axis]),
            });
            return true;
        };
        let gap = now.duration_since(current.last_event);
        current.last_event = now;
        if gap >= WHEEL_QUIET_GAP {
            current.axis = axis;
            current.recent = VecDeque::from([axis]);
            return true;
        }
        if gap >= WHEEL_NOTCH_GAP {
            return true;
        }
        current.recent.push_back(axis);
        while current.recent.len() > TAKEOVER_RUN {
            current.recent.pop_front();
        }
        if axis == current.axis {
            return true;
        }
        let run = current.recent.iter().filter(|seen| **seen == axis).count();
        if run >= TAKEOVER_RUN - 1 {
            current.axis = axis;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WheelAxis::{Horizontal as H, Vertical as V};

    /// Feeds `(gap_ms, axis)` pairs, each arriving that long after the last.
    fn feed(lock: &mut Option<WheelAxisLock>, events: &[(u64, WheelAxis)]) -> Vec<bool> {
        let mut at = Instant::now();
        events
            .iter()
            .map(|(gap, axis)| {
                at += Duration::from_millis(*gap);
                WheelAxisLock::admit(lock, *axis, at)
            })
            .collect()
    }

    #[test]
    fn strays_inside_a_dense_scroll_are_dropped() {
        let mut lock = None;
        let admitted = feed(
            &mut lock,
            &[
                (0, V),
                (5, V),
                (5, V),
                (5, H),
                (5, V),
                (5, V),
                (5, H),
                (5, V),
            ],
        );
        assert_eq!(admitted, [true, true, true, false, true, true, false, true]);
    }

    #[test]
    fn a_swipe_takes_over_from_a_scroll_while_its_momentum_still_arrives() {
        let mut lock = None;
        // Dense vertical scroll, then momentum at 30ms while a dense swipe starts.
        let admitted = feed(
            &mut lock,
            &[
                (0, V),
                (5, V),
                (5, V),
                (30, V),
                (5, H),
                (5, H),
                (5, H),
                (15, V),
                (5, H),
                (5, V),
            ],
        );
        assert_eq!(
            admitted,
            [true, true, true, true, false, false, false, true, true, false],
            "four of the last five events are the swipe, so it takes the lock and later momentum is dropped"
        );
    }

    #[test]
    fn a_scroll_takes_over_from_a_swipe_the_same_way() {
        let mut lock = None;
        let admitted = feed(
            &mut lock,
            &[
                (0, H),
                (5, H),
                (5, H),
                (5, V),
                (5, V),
                (5, V),
                (5, V),
                (5, H),
            ],
        );
        assert_eq!(
            admitted,
            [true, true, true, false, false, false, true, false]
        );
    }

    #[test]
    fn sparse_notches_bypass_the_lock() {
        let mut lock = None;
        // Wheel notches: vertical, then a horizontal tilt without a pause.
        let admitted = feed(&mut lock, &[(0, V), (80, V), (80, H), (80, H)]);
        assert_eq!(admitted, [true, true, true, true]);
    }

    #[test]
    fn a_quiet_gap_starts_a_new_gesture_on_its_own_axis() {
        let mut lock = None;
        let quiet = WHEEL_QUIET_GAP.as_millis() as u64;
        let admitted = feed(&mut lock, &[(0, V), (5, V), (quiet, H), (5, H), (5, V)]);
        assert_eq!(admitted, [true, true, true, true, false]);
    }
}
