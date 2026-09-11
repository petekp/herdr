use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Weighted wheel events in one direction needed to switch tabs. A trackpad
/// swipe on macOS delivers roughly 150 events, so this commits about a third
/// of the way through a normal swipe.
pub(super) const TAB_SWIPE_THRESHOLD: u32 = 48;
/// Weighted events a swipe accumulates before the fill starts to move, like
/// the minimum distance before a touch pan gesture begins. A few stray
/// sideways events during a vertical scroll never show; a real swipe loses
/// nothing visible.
pub(super) const TAB_SWIPE_DEAD_ZONE: u32 = 8;
/// Quiet time after which an unfinished swipe snaps back, and the fallback
/// after which a finished swipe stops swallowing the tail of its burst.
pub(super) const TAB_SWIPE_IDLE: Duration = Duration::from_millis(220);
/// How long the fill takes to slide onto the target or back to the origin.
pub(super) const TAB_SWIPE_SETTLE: Duration = Duration::from_millis(120);
const TAB_SWIPE_FRAME: Duration = Duration::from_millis(16);
/// Events closer together than this only come from trackpads, which emit
/// several per frame. Discrete mouse notches never arrive this fast.
const FAST_GAP: Duration = Duration::from_millis(20);
/// Fast gaps seen before the gesture counts as trackpad input. One is not
/// enough: a busy client can read two wheel notches in one batch.
const TRACKPAD_FAST_EVENTS: u32 = 2;
/// Gap at or above which an event counts as a discrete notch. Gaps between
/// `FAST_GAP` and this are ambiguous, a fast wheel flick or a moderate
/// trackpad swipe, and weigh 1 so the rarer failure is a switch that needs
/// more input.
const SPARSE_GAP: Duration = super::wheel_axis::WHEEL_NOTCH_GAP;
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
    /// Neighbor the fill is moving toward. `None` while the swipe is at rest.
    pub(super) target: Option<TabSwipeTarget>,
    /// How far the fill has moved from the origin toward the target, 0 to 1.
    /// Stays at 0 inside the dead zone.
    pub(super) progress: f32,
    /// Net weighted wheel events so far. Positive moves toward the next tab.
    steps: i32,
    /// When the last wheel event arrived, or when the swipe began.
    last_input: Instant,
    last_event: Option<Instant>,
    /// The read the last event arrived in.
    batch: Option<InputBatch>,
    fast_events: u32,
    /// Event times within the last two resume windows.
    recent: VecDeque<Instant>,
    phase: TabSwipePhase,
}

/// The neighbor a swipe is moving toward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TabSwipeTarget {
    pub(super) tab_id: String,
    /// The target sits at the far end of the strip, so the fill wraps around.
    pub(super) wraps: bool,
}

/// Events read from the terminal together. They share one timestamp but
/// arrived over the gap since the previous read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputBatch {
    /// The previous read, which this batch's events followed.
    since: Instant,
    len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TabSwipePhase {
    /// Counting wheel events toward the threshold.
    Tracking,
    /// Went idle before the threshold: the fill slides back to the origin.
    SnappingBack { from: f32, started: Instant },
    /// Reached the threshold: the fill slides onto the target.
    Landing {
        from: f32,
        started: Instant,
        commit: TabSwipeCommit,
    },
    /// Landed; swallow the rest of the burst that committed.
    Cooldown { commit: TabSwipeCommit },
}

/// The moment a swipe reached the threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TabSwipeCommit {
    /// Sign of the step count that committed.
    direction: i32,
    at: Instant,
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

impl<'a> TabSwipeNeighbors<'a> {
    /// Neighbors of `tab_ids[origin]`. With two or more tabs the strip wraps,
    /// so both neighbors always exist; a lone tab has none.
    pub(super) fn around(tab_ids: &'a [String], origin: usize) -> Self {
        let count = tab_ids.len();
        if count < 2 || origin >= count {
            return Self::default();
        }
        let previous_wraps = origin == 0;
        let next_wraps = origin + 1 == count;
        let previous = if previous_wraps {
            count - 1
        } else {
            origin - 1
        };
        let next = if next_wraps { 0 } else { origin + 1 };
        Self {
            previous: tab_ids.get(previous).map(String::as_str),
            next: tab_ids.get(next).map(String::as_str),
            previous_wraps,
            next_wraps,
        }
    }
}

impl TabSwipe {
    pub(super) fn begin(workspace_id: String, origin_tab_id: String, now: Instant) -> Self {
        Self {
            workspace_id,
            origin_tab_id,
            target: None,
            progress: 0.0,
            steps: 0,
            last_input: now,
            last_event: None,
            batch: None,
            fast_events: 0,
            recent: VecDeque::new(),
            phase: TabSwipePhase::Tracking,
        }
    }

    /// -1 toward the previous tab, 1 toward the next, 0 at rest.
    pub(super) fn direction(&self) -> i32 {
        self.steps.signum()
    }

    /// Feeds one wheel event. A direction without a neighbor makes no
    /// progress; the strip wraps only when the caller supplies the far tab as
    /// that neighbor.
    pub(super) fn push(
        &mut self,
        delta: i32,
        neighbors: TabSwipeNeighbors<'_>,
        now: Instant,
    ) -> TabSwipePush {
        let gap = self.last_event.map(|last| now.duration_since(last));
        self.record_event(gap, now);
        if gap.is_some_and(|gap| gap < FAST_GAP) {
            self.fast_events += 1;
        }
        let discrete_notch = !self.trackpad_input() && gap.is_some_and(|gap| gap >= SPARSE_GAP);
        let weight = if discrete_notch {
            SPARSE_EVENT_WEIGHT
        } else {
            1
        };
        match self.phase {
            TabSwipePhase::Tracking => {}
            TabSwipePhase::SnappingBack { .. } => {
                // Resume from wherever the snap-back animation currently is.
                let sign = if self.steps < 0 { -1 } else { 1 };
                self.steps = Self::steps_for_progress(self.progress) as i32 * sign;
                self.phase = TabSwipePhase::Tracking;
            }
            TabSwipePhase::Landing { commit, .. } | TabSwipePhase::Cooldown { commit } => {
                // A notch cannot be momentum: nothing coasts at wheel rates.
                // It starts the next swipe at once, so a wheel keeps switching
                // tabs for as long as it keeps turning.
                if discrete_notch || self.fresh_swipe(delta, commit, now) {
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
        let (neighbor, wraps) = match steps.signum() {
            1 => (neighbors.next, neighbors.next_wraps),
            -1 => (neighbors.previous, neighbors.previous_wraps),
            _ => (None, false),
        };
        self.target = neighbor.map(|tab_id| TabSwipeTarget {
            tab_id: tab_id.to_owned(),
            wraps,
        });
        let magnitude = steps.unsigned_abs();
        if magnitude < TAB_SWIPE_THRESHOLD {
            self.progress = Self::progress_for_steps(magnitude);
            return TabSwipePush::Continue;
        }
        // Steps only move toward a neighbor that exists, so a threshold-sized
        // count always has a target.
        let Some(target) = &self.target else {
            return TabSwipePush::Continue;
        };
        self.phase = TabSwipePhase::Landing {
            from: self.progress,
            started: now,
            commit: TabSwipeCommit {
                direction: steps.signum(),
                at: now,
            },
        };
        TabSwipePush::Commit(target.tab_id.clone())
    }

    /// Fill position for a step count: nothing inside the dead zone, then a
    /// straight line from 0 at its edge to 1 at the threshold.
    fn progress_for_steps(magnitude: u32) -> f32 {
        magnitude.saturating_sub(TAB_SWIPE_DEAD_ZONE) as f32
            / (TAB_SWIPE_THRESHOLD - TAB_SWIPE_DEAD_ZONE) as f32
    }

    /// Inverse of `progress_for_steps` for resuming a snap-back.
    fn steps_for_progress(progress: f32) -> u32 {
        if progress <= 0.0 {
            return 0;
        }
        TAB_SWIPE_DEAD_ZONE
            + (progress * (TAB_SWIPE_THRESHOLD - TAB_SWIPE_DEAD_ZONE) as f32).round() as u32
    }

    /// Records the event's time for the rate test. Events read from the
    /// terminal together share a timestamp but arrived over the gap since the
    /// previous read, so a batch is spread evenly across that gap. A stalled
    /// client's backlog then looks like the steady input it was, not a burst.
    fn record_event(&mut self, gap: Option<Duration>, now: Instant) {
        match (gap, &mut self.batch) {
            (Some(Duration::ZERO), Some(batch)) => {
                batch.len += 1;
                let InputBatch { since, len } = *batch;
                let span = now.duration_since(since);
                self.recent
                    .truncate(self.recent.len().saturating_sub(len - 1));
                self.recent
                    .extend((1..=len).map(|k| since + span * k as u32 / len as u32));
            }
            _ => {
                self.batch = Some(InputBatch {
                    since: self.last_event.unwrap_or(now),
                    len: 1,
                });
                self.recent.push_back(now);
            }
        }
        self.last_event = Some(now);
        self.last_input = now;
        while self
            .recent
            .front()
            .is_some_and(|first| now.duration_since(*first) > RESUME_WINDOW * 2)
        {
            self.recent.pop_front();
        }
    }

    fn trackpad_input(&self) -> bool {
        self.fast_events >= TRACKPAD_FAST_EVENTS
    }

    /// Whether a trackpad event arriving after a commit belongs to a new swipe
    /// rather than the momentum tail of the one that committed.
    fn fresh_swipe(&self, delta: i32, commit: TabSwipeCommit, now: Instant) -> bool {
        if now.duration_since(commit.at) < RESUME_MIN_AGE {
            return false;
        }
        if delta.signum() != commit.direction {
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
            TabSwipePhase::Tracking if idle && self.progress == 0.0 => TabSwipeTick {
                repaint: false,
                finished: true,
            },
            TabSwipePhase::Tracking if idle => {
                self.phase = TabSwipePhase::SnappingBack {
                    from: self.progress,
                    started: now,
                };
                TabSwipeTick::default()
            }
            TabSwipePhase::Tracking => TabSwipeTick::default(),
            TabSwipePhase::SnappingBack { from, started } => {
                let (repaint, arrived) = self.settle(from, 0.0, started, now);
                TabSwipeTick {
                    repaint,
                    finished: arrived,
                }
            }
            TabSwipePhase::Landing {
                from,
                started,
                commit,
            } => {
                let (repaint, arrived) = self.settle(from, 1.0, started, now);
                if arrived {
                    self.phase = TabSwipePhase::Cooldown { commit };
                }
                TabSwipeTick {
                    repaint,
                    finished: arrived && idle,
                }
            }
            TabSwipePhase::Cooldown { .. } => TabSwipeTick {
                repaint: false,
                finished: idle,
            },
        }
    }

    /// Moves the fill along an ease-out curve from `from` toward `to`.
    /// Returns whether it moved and whether it has arrived.
    fn settle(&mut self, from: f32, to: f32, started: Instant, now: Instant) -> (bool, bool) {
        let elapsed = now.duration_since(started).as_secs_f32();
        let t = (elapsed / TAB_SWIPE_SETTLE.as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        let next = if t >= 1.0 {
            to
        } else {
            from + (to - from) * eased
        };
        let repaint = next != self.progress;
        self.progress = next;
        (repaint, t >= 1.0)
    }

    /// When the client loop should next call `tick`.
    pub(super) fn next_wake(&self, now: Instant) -> Instant {
        match self.phase {
            TabSwipePhase::SnappingBack { .. } | TabSwipePhase::Landing { .. } => {
                now + TAB_SWIPE_FRAME
            }
            TabSwipePhase::Tracking | TabSwipePhase::Cooldown { .. } => {
                self.last_input + TAB_SWIPE_IDLE
            }
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

    fn target_id(swipe: &TabSwipe) -> Option<&str> {
        swipe.target.as_ref().map(|target| target.tab_id.as_str())
    }

    fn wraps(swipe: &TabSwipe) -> bool {
        swipe.target.as_ref().is_some_and(|target| target.wraps)
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
        assert!(matches!(
            swipe.phase,
            TabSwipePhase::Landing { commit, .. } if commit.direction == 1
        ));
    }

    #[test]
    fn progress_tracks_the_event_count_past_the_dead_zone() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_DEAD_ZONE, now);
        assert_eq!(swipe.progress, 0.0, "the dead zone draws nothing");
        assert_eq!(target_id(&swipe), Some("tab_next"));
        let quarter = (TAB_SWIPE_THRESHOLD - TAB_SWIPE_DEAD_ZONE) / 4;
        trackpad(&mut swipe, 1, quarter, at);
        assert!((swipe.progress - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_few_stray_events_finish_quietly_when_idle() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, 3, now);
        assert_eq!(swipe.progress, 0.0);
        let tick = swipe.tick(at + TAB_SWIPE_IDLE);
        assert!(tick.finished && !tick.repaint, "no snap-back to animate");
    }

    #[test]
    fn reversing_direction_moves_the_fill_back_toward_the_origin() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_DEAD_ZONE + 2, now);
        let before = swipe.progress;
        let (at, _) = trackpad(&mut swipe, -1, 1, at);
        assert!(swipe.progress > 0.0 && swipe.progress < before);
        trackpad(&mut swipe, -1, TAB_SWIPE_DEAD_ZONE + 3, at);
        assert_eq!(target_id(&swipe), Some("tab_prev"));
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
        assert_eq!(swipe.target, None);
        assert_eq!(swipe.phase, TabSwipePhase::Tracking);
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
        assert_eq!(target_id(&swipe), Some("tab_last"));
        assert_eq!(swipe.direction(), -1);
        assert!(wraps(&swipe));
        swipe.push(1, first_tab, now);
        swipe.push(1, first_tab, now);
        assert_eq!(target_id(&swipe), Some("tab_second"));
        assert!(!wraps(&swipe));
    }

    #[test]
    fn neighbors_wrap_around_the_strip() {
        let tabs = ["a", "b", "c"].map(String::from);
        let first = TabSwipeNeighbors::around(&tabs, 0);
        assert_eq!((first.previous, first.next), (Some("c"), Some("b")));
        assert!(first.previous_wraps && !first.next_wraps);
        let middle = TabSwipeNeighbors::around(&tabs, 1);
        assert_eq!((middle.previous, middle.next), (Some("a"), Some("c")));
        assert!(!middle.previous_wraps && !middle.next_wraps);
        let last = TabSwipeNeighbors::around(&tabs, 2);
        assert_eq!((last.previous, last.next), (Some("b"), Some("a")));
        assert!(!last.previous_wraps && last.next_wraps);
    }

    #[test]
    fn a_lone_tab_has_no_neighbors_and_a_pair_shares_one() {
        let lone = ["a"].map(String::from);
        let neighbors = TabSwipeNeighbors::around(&lone, 0);
        assert_eq!((neighbors.previous, neighbors.next), (None, None));
        let pair = ["a", "b"].map(String::from);
        let first = TabSwipeNeighbors::around(&pair, 0);
        assert_eq!((first.previous, first.next), (Some("b"), Some("b")));
        assert!(first.previous_wraps && !first.next_wraps);
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
    fn a_second_swipe_whose_first_events_share_a_read_still_restarts() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (mut at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        // Late momentum tail, with the gaps grown to 56ms.
        for gap in (8..=56).step_by(2) {
            at += Duration::from_millis(gap);
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
            swipe.tick(at);
        }
        // Fingers come back 58ms later, and the client reads the first two
        // events of the new swipe together, so they share a timestamp.
        at += Duration::from_millis(58);
        assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
        assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
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
    fn wheel_notches_keep_switching_tabs_without_a_pause() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let mut at = now;
        let mut results = Vec::new();
        for _ in 0..7 {
            results.push(swipe.push(1, neighbors(), at));
            at += Duration::from_millis(80);
        }
        assert_eq!(
            results.last(),
            Some(&TabSwipePush::Commit("tab_next".into()))
        );
        // The next notch lands during the settle and starts a new swipe
        // instead of being swallowed as momentum, in either direction.
        assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Restart);
        at += Duration::from_millis(80);
        assert_eq!(swipe.push(-1, neighbors(), at), TabSwipePush::Restart);
    }

    #[test]
    fn one_batched_notch_pair_keeps_wheel_weighting() {
        // A busy client read two notches in one batch, so they share an instant.
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        assert_eq!(swipe.push(1, neighbors(), now), TabSwipePush::Continue);
        assert_eq!(swipe.push(1, neighbors(), now), TabSwipePush::Continue);
        let mut at = now;
        let mut notches = 2;
        loop {
            at += Duration::from_millis(80);
            notches += 1;
            if swipe.push(1, neighbors(), at) == TabSwipePush::Commit("tab_next".into()) {
                break;
            }
            assert!(notches < 10, "wheel needs too many notches");
        }
        assert_eq!(notches, 8);
    }

    #[test]
    fn a_stalled_client_batching_the_momentum_tail_does_not_restart() {
        let now = Instant::now();
        let mut swipe = TabSwipe::begin("ws".into(), "tab_origin".into(), now);
        let (mut at, _) = trackpad(&mut swipe, 1, TAB_SWIPE_THRESHOLD, now);
        // Momentum at 10ms gaps, processed on time.
        for _ in 0..16 {
            at += Duration::from_millis(10);
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
            swipe.tick(at);
        }
        // The client stalls for 200ms, then processes the 20 tail events that
        // queued up meanwhile as one batch, all stamped with the same instant.
        at += Duration::from_millis(200);
        for _ in 0..20 {
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
        }
        // The rest of the tail is processed on time again.
        for _ in 0..10 {
            at += Duration::from_millis(10);
            assert_eq!(swipe.push(1, neighbors(), at), TabSwipePush::Continue);
        }
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
