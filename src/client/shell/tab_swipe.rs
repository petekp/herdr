use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Weighted wheel events in one direction needed to switch tabs. A trackpad
/// swipe on macOS delivers roughly 150 events, so this commits about a third
/// of the way through a normal swipe.
pub(super) const TAB_SWIPE_THRESHOLD: u32 = 48;
/// Quiet time after which an unfinished swipe snaps back, and the fallback
/// after which a finished swipe stops swallowing the tail of its burst.
pub(super) const TAB_SWIPE_IDLE: Duration = Duration::from_millis(220);
/// How long the fill takes to slide onto the target or back to the origin.
pub(super) const TAB_SWIPE_SETTLE: Duration = Duration::from_millis(120);
const TAB_SWIPE_FRAME: Duration = Duration::from_millis(16);
/// Events closer together than this only come from trackpads, which emit
/// several per frame. Discrete mouse notches never arrive this fast.
const FAST_GAP: Duration = Duration::from_millis(20);
/// Gap at or above which an event counts as a discrete notch.
const SPARSE_GAP: Duration = Duration::from_millis(40);
/// Weight of a discrete notch, so a mouse wheel switches in a handful of clicks.
const SPARSE_EVENT_WEIGHT: i32 = 8;
/// A committed swipe ignores input this long so the burst that committed it
/// cannot immediately start another.
const RESUME_MIN_AGE: Duration = Duration::from_millis(150);
const RESUME_WINDOW: Duration = Duration::from_millis(100);
/// Events in the last window needed to treat the input as a fresh swipe...
const RESUME_MIN_EVENTS: usize = 12;
/// ...and how much faster than the previous window it has to be. Momentum
/// only decays, so a rise this sharp means the fingers came back.
const RESUME_RATIO: f32 = 2.5;

/// A wheel-driven tab switch in progress. Wheel events carry no distance, so
/// the gesture counts them and commits once one direction reaches the threshold.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TabSwipe {
    pub(super) workspace_id: String,
    /// Tab that was focused when the swipe began.
    pub(super) origin_tab_id: String,
    /// Neighbor the fill is currently moving toward, if the direction has one.
    pub(super) target_tab_id: Option<String>,
    /// Sign of `steps`: -1 toward the previous tab, 1 toward the next, 0 at rest.
    pub(super) direction: i32,
    /// The target sits at the far end of the strip, so the fill wraps around.
    pub(super) wraps: bool,
    /// How far the fill has moved from the origin toward the target, 0 to 1.
    pub(super) progress: f32,
    /// Net weighted wheel events so far. Positive moves toward the next tab.
    steps: i32,
    /// Direction that committed, as the sign of `steps` at that moment.
    committed: Option<i32>,
    commit_at: Option<Instant>,
    last_input: Instant,
    last_event: Option<Instant>,
    fast_events: u32,
    /// Event times within the last two resume windows.
    recent: VecDeque<Instant>,
    phase: TabSwipePhase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TabSwipePhase {
    Tracking,
    Settling {
        from: f32,
        to: f32,
        started: Instant,
    },
    /// Committed and settled; swallow the rest of the burst.
    Cooldown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TabSwipePush {
    /// The fill moved, or the event was swallowed.
    Continue,
    /// The threshold was reached; focus this tab.
    Commit(String),
    /// A fresh swipe arrived while the previous one was still finishing.
    /// The caller starts a new gesture from the committed target and replays
    /// the event into it.
    Restart,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TabSwipeTick {
    pub(super) repaint: bool,
    pub(super) finished: bool,
}

/// Neighbors of the origin tab, in tab-strip order.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct TabSwipeNeighbors<'a> {
    pub(super) previous: Option<&'a str>,
    pub(super) next: Option<&'a str>,
    /// `previous` is the last tab, reached by wrapping past the first.
    pub(super) previous_wraps: bool,
    /// `next` is the first tab, reached by wrapping past the last.
    pub(super) next_wraps: bool,
}

impl TabSwipe {
    pub(super) fn begin(workspace_id: String, origin_tab_id: String, now: Instant) -> Self {
        Self {
            workspace_id,
            origin_tab_id,
            target_tab_id: None,
            direction: 0,
            wraps: false,
            progress: 0.0,
            steps: 0,
            committed: None,
            commit_at: None,
            last_input: now,
            last_event: None,
            fast_events: 0,
            recent: VecDeque::new(),
            phase: TabSwipePhase::Tracking,
        }
    }

    /// Feeds one wheel event. Directions without a neighbor make no progress,
    /// so the strip never wraps.
    pub(super) fn push(
        &mut self,
        delta: i32,
        neighbors: TabSwipeNeighbors<'_>,
        now: Instant,
    ) -> TabSwipePush {
        let gap = self.last_event.map(|last| now.duration_since(last));
        self.last_event = Some(now);
        self.last_input = now;
        self.recent.push_back(now);
        while self
            .recent
            .front()
            .is_some_and(|first| now.duration_since(*first) > RESUME_WINDOW * 2)
        {
            self.recent.pop_front();
        }
        if gap.is_some_and(|gap| gap < FAST_GAP) {
            self.fast_events += 1;
        }
        let weight = if self.fast_events == 0 && gap.is_some_and(|gap| gap >= SPARSE_GAP) {
            SPARSE_EVENT_WEIGHT
        } else {
            1
        };
        match self.phase {
            TabSwipePhase::Tracking => {}
            TabSwipePhase::Settling { .. } if self.committed.is_none() => {
                // Resume from wherever the snap-back animation currently is.
                let sign = if self.steps < 0 { -1 } else { 1 };
                self.steps = (self.progress * TAB_SWIPE_THRESHOLD as f32).round() as i32 * sign;
                self.phase = TabSwipePhase::Tracking;
            }
            TabSwipePhase::Settling { .. } | TabSwipePhase::Cooldown => {
                if self.fresh_swipe(delta, now) {
                    return TabSwipePush::Restart;
                }
                return TabSwipePush::Continue;
            }
        }
        let mut steps = self.steps + delta.signum() * weight;
        if neighbors.next.is_none() {
            steps = steps.min(0);
        }
        if neighbors.previous.is_none() {
            steps = steps.max(0);
        }
        self.steps = steps;
        self.direction = steps.signum();
        (self.target_tab_id, self.wraps) = match steps.signum() {
            1 => (neighbors.next.map(str::to_owned), neighbors.next_wraps),
            -1 => (
                neighbors.previous.map(str::to_owned),
                neighbors.previous_wraps,
            ),
            _ => (None, false),
        };
        let magnitude = steps.unsigned_abs();
        if magnitude >= TAB_SWIPE_THRESHOLD {
            self.committed = Some(steps.signum());
            self.commit_at = Some(now);
            self.phase = TabSwipePhase::Settling {
                from: self.progress,
                to: 1.0,
                started: now,
            };
            return match self.target_tab_id.clone() {
                Some(target) => TabSwipePush::Commit(target),
                None => TabSwipePush::Continue,
            };
        }
        self.progress = magnitude as f32 / TAB_SWIPE_THRESHOLD as f32;
        TabSwipePush::Continue
    }

    /// Whether an event arriving after a commit belongs to a new swipe rather
    /// than the momentum tail of the one that committed.
    fn fresh_swipe(&self, delta: i32, now: Instant) -> bool {
        let (Some(direction), Some(commit_at)) = (self.committed, self.commit_at) else {
            return false;
        };
        if now.duration_since(commit_at) < RESUME_MIN_AGE {
            return false;
        }
        if delta.signum() != direction {
            return true;
        }
        let recent = self
            .recent
            .iter()
            .filter(|at| now.duration_since(**at) <= RESUME_WINDOW)
            .count();
        let older = self.recent.len() - recent;
        recent >= RESUME_MIN_EVENTS && recent as f32 >= RESUME_RATIO * older as f32
    }

    pub(super) fn tick(&mut self, now: Instant) -> TabSwipeTick {
        let idle = now.duration_since(self.last_input) >= TAB_SWIPE_IDLE;
        match self.phase {
            TabSwipePhase::Tracking if idle => {
                if self.progress == 0.0 {
                    return TabSwipeTick {
                        repaint: false,
                        finished: true,
                    };
                }
                self.phase = TabSwipePhase::Settling {
                    from: self.progress,
                    to: 0.0,
                    started: now,
                };
                TabSwipeTick::default()
            }
            TabSwipePhase::Tracking => TabSwipeTick::default(),
            TabSwipePhase::Settling { from, to, started } => {
                let elapsed = now.duration_since(started).as_secs_f32();
                let t = (elapsed / TAB_SWIPE_SETTLE.as_secs_f32()).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - t) * (1.0 - t);
                let next = from + (to - from) * eased;
                let repaint = next != self.progress;
                self.progress = next;
                if t < 1.0 {
                    return TabSwipeTick {
                        repaint,
                        finished: false,
                    };
                }
                if self.committed.is_some() {
                    self.phase = TabSwipePhase::Cooldown;
                    TabSwipeTick {
                        repaint,
                        finished: idle,
                    }
                } else {
                    TabSwipeTick {
                        repaint: true,
                        finished: true,
                    }
                }
            }
            TabSwipePhase::Cooldown => TabSwipeTick {
                repaint: false,
                finished: idle,
            },
        }
    }

    /// When the client loop should next call `tick`.
    pub(super) fn next_wake(&self, now: Instant) -> Instant {
        match self.phase {
            TabSwipePhase::Settling { .. } => now + TAB_SWIPE_FRAME,
            TabSwipePhase::Tracking | TabSwipePhase::Cooldown => self.last_input + TAB_SWIPE_IDLE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neighbors() -> TabSwipeNeighbors<'static> {
        TabSwipeNeighbors {
            previous: Some("tab_prev"),
            next: Some("tab_next"),
            ..TabSwipeNeighbors::default()
        }
    }

    /// Pushes `count` events spaced like a trackpad, returning the last time.
    fn trackpad(
        swipe: &mut TabSwipe,
        delta: i32,
        count: u32,
        mut at: Instant,
    ) -> (Instant, Vec<TabSwipePush>) {
        let mut results = Vec::new();
        for _ in 0..count {
            results.push(swipe.push(delta, neighbors(), at));
            at += Duration::from_millis(4);
        }
        (at, results)
    }

    #[test]
    fn commits_toward_next_after_threshold() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (_, results) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        let (last, rest) = results.split_last().expect("events");
        assert!(rest.iter().all(|result| *result == TabSwipePush::Continue));
        assert_eq!(*last, TabSwipePush::Commit("tab_next".into()));
        assert_eq!(swipe.committed, Some(1));
    }

    #[test]
    fn progress_tracks_the_event_count() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD / 4, now);
        assert!((swipe.progress - 0.25).abs() < 1e-6);
        assert_eq!(swipe.target_tab_id.as_deref(), Some("tab_next"));
    }

    #[test]
    fn reversing_direction_moves_the_fill_back_toward_the_origin() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, 2, now);
        let (at, _) = trackpad(&mut swipe, -1, 1, at);
        assert!((swipe.progress - 1.0 / TAB_SWIPE_THRESHOLD as f32).abs() < 1e-6);
        trackpad(&mut swipe, -1, 2, at);
        assert_eq!(swipe.target_tab_id.as_deref(), Some("tab_prev"));
    }

    #[test]
    fn missing_neighbor_makes_no_progress() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let first_tab = TabSwipeNeighbors {
            previous: None,
            next: Some("tab_next"),
            ..TabSwipeNeighbors::default()
        };
        for _ in 0..TAB_SWIPE_THRESHOLD * 2 {
            assert_eq!(swipe.push(-1, first_tab, now), TabSwipePush::Continue);
        }
        assert_eq!(swipe.progress, 0.0);
        assert_eq!(swipe.target_tab_id, None);
        assert_eq!(swipe.committed, None);
    }

    #[test]
    fn wrapping_neighbor_is_remembered_with_the_direction() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_first".into(), now);
        let first_tab = TabSwipeNeighbors {
            previous: Some("tab_last"),
            next: Some("tab_second"),
            previous_wraps: true,
            next_wraps: false,
        };
        swipe.push(-1, first_tab, now);
        assert_eq!(swipe.target_tab_id.as_deref(), Some("tab_last"));
        assert_eq!(swipe.direction, -1);
        assert!(swipe.wraps);
        swipe.push(1, first_tab, now);
        swipe.push(1, first_tab, now);
        assert_eq!(swipe.target_tab_id.as_deref(), Some("tab_second"));
        assert!(!swipe.wraps);
    }

    #[test]
    fn idle_before_threshold_snaps_back_and_finishes() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD / 3, now);
        assert_eq!(swipe.tick(at + TAB_SWIPE_IDLE / 2), TabSwipeTick::default());
        let idle_at = at + TAB_SWIPE_IDLE;
        assert!(!swipe.tick(idle_at).finished);
        let mid = swipe.tick(idle_at + TAB_SWIPE_SETTLE / 2);
        assert!(mid.repaint && !mid.finished);
        assert!(swipe.progress > 0.0 && swipe.progress < 1.0 / 3.0);
        let done = swipe.tick(idle_at + TAB_SWIPE_SETTLE);
        assert!(done.finished);
        assert_eq!(swipe.progress, 0.0);
    }

    #[test]
    fn momentum_tail_after_a_commit_is_swallowed() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (mut at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        // Momentum: same direction, gaps growing from 8ms to 60ms over ~700ms.
        for gap in (8..=60).step_by(2) {
            at += Duration::from_millis(gap);
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
            swipe.tick(at);
        }
        assert_eq!(swipe.progress, 1.0);
        assert!(swipe.tick(at + TAB_SWIPE_IDLE).finished);
    }

    #[test]
    fn a_reversed_swipe_after_a_commit_restarts() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        assert_eq!(swipe.push(-1, neighbors(), at), TabSwipePush::Continue);
        let later = at + RESUME_MIN_AGE;
        assert_eq!(swipe.push(-1, neighbors(), later), TabSwipePush::Restart);
    }

    #[test]
    fn a_faster_same_direction_burst_during_momentum_restarts() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (mut at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        // Sparse momentum tail for 300ms.
        for _ in 0..10 {
            at += Duration::from_millis(30);
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
        }
        // Fingers come back: a dense burst.
        let (_, results) = trackpad(&mut swipe, 1, 20, at + Duration::from_millis(4));
        assert!(results.contains(&TabSwipePush::Restart));
    }

    #[test]
    fn discrete_wheel_notches_switch_in_a_few_clicks() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let mut at = now;
        let mut notches = 0;
        loop {
            notches += 1;
            let result = swipe.push(1, neighbors(), at);
            if result == TabSwipePush::Commit("tab_next".into()) {
                break;
            }
            assert!(notches < 10, "wheel needs too many notches");
            at += Duration::from_millis(80);
        }
        assert_eq!(notches, 7);
    }

    #[test]
    fn wake_follows_the_phase() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        assert_eq!(swipe.next_wake(now), now + TAB_SWIPE_IDLE);
        let (at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        assert_eq!(swipe.next_wake(at), at + TAB_SWIPE_FRAME);
    }
}
