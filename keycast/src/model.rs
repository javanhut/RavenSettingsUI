//! The strokes on screen, and when each one comes and goes.
//!
//! Free of drawing and of devices so the rules can be tested: which press
//! joins a group of caps already showing, which one starts a new group, and
//! when a group leaves.

use std::time::{Duration, Instant};

use crate::keys::{self, Kind, Modifier};

/// How long each movement takes. A press is quick so the cap is down while
/// the finger is; a release is a little slower so the eye catches it.
pub const ENTER: Duration = Duration::from_millis(180);
pub const PRESS: Duration = Duration::from_millis(70);
pub const RELEASE: Duration = Duration::from_millis(160);
pub const RIPPLE: Duration = Duration::from_millis(340);
pub const POP: Duration = Duration::from_millis(220);
pub const EXIT: Duration = Duration::from_millis(220);

/// Most groups on screen at once; one more pushes the oldest out.
pub const MAX_GROUPS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every key.
    All,
    /// Only what is pressed with Ctrl, Alt or Super, and keys that type no
    /// text: nothing typed into a document, or a password field, shows.
    Shortcuts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub mode: Mode,
    pub show_mouse: bool,
    /// How long a group stays once all its keys are up.
    pub hide_after: Duration,
}

#[derive(Debug, Clone)]
pub struct Cap {
    pub code: u16,
    pub label: &'static str,
    pub modifier: Option<Modifier>,
    pub down: bool,
    /// When `down` last changed.
    pub changed: Instant,
    /// When the key last went down, for the ripple.
    pub pressed: Instant,
}

impl Cap {
    /// A key going down now.
    fn pressed(code: u16, now: Instant) -> Option<Cap> {
        let (_, label) = keys::describe(code)?;
        Some(Cap {
            code,
            label,
            modifier: keys::modifier(code),
            down: true,
            changed: now,
            pressed: now,
        })
    }

    /// A key that was already down before the group it is joining existed:
    /// Ctrl, still held from Ctrl+C, in the Ctrl+V after it. Drawn down from
    /// the first frame, with no press to animate.
    fn held(code: u16, now: Instant) -> Option<Cap> {
        let past = now.checked_sub(Duration::from_secs(5)).unwrap_or(now);
        Cap::pressed(code, now).map(|c| Cap {
            changed: past,
            pressed: past,
            ..c
        })
    }
}

#[derive(Debug, Clone)]
pub struct Group {
    pub id: u64,
    pub caps: Vec<Cap>,
    /// Times this stroke was pressed in a row.
    pub count: u32,
    pub born: Instant,
    /// The last press that made or repeated this group.
    pub bumped: Instant,
    /// When the last of its keys came up; `None` while any is held.
    pub released: Option<Instant>,
    pub leaving: Option<Instant>,
}

impl Group {
    fn modifier_only(&self) -> bool {
        self.caps.iter().all(|c| c.modifier.is_some())
    }

    fn same_stroke(&self, caps: &[Cap]) -> bool {
        self.caps.len() == caps.len() && self.caps.iter().zip(caps).all(|(a, b)| a.label == b.label)
    }

    /// Put `code` down again, for a repeat.
    fn repeat(&mut self, code: u16, now: Instant) {
        self.count += 1;
        self.bumped = now;
        self.released = None;
        if let Some(cap) = self.caps.iter_mut().find(|c| c.code == code) {
            cap.down = true;
            cap.changed = now;
            cap.pressed = now;
        }
    }

    /// Replace the caps with `caps`, keeping the timing of any already here
    /// so a cap that is down does not press down a second time.
    fn grow(&mut self, caps: Vec<Cap>, now: Instant) {
        let merged = caps
            .into_iter()
            .map(|new| {
                self.caps
                    .iter()
                    .find(|old| old.label == new.label && old.down)
                    .cloned()
                    .unwrap_or(new)
            })
            .collect();
        self.caps = merged;
        self.bumped = now;
        self.released = None;
    }
}

#[derive(Debug, Default)]
pub struct Strokes {
    pub groups: Vec<Group>,
    /// Modifier keys down right now, by physical code.
    held: Vec<(u16, Modifier)>,
    /// The group still taking keys: made while modifiers are held, so the
    /// next key pressed with them lands in it rather than beside it.
    open: Option<u64>,
    next_id: u64,
}

impl Strokes {
    pub fn press(&mut self, code: u16, now: Instant, rules: &Rules) {
        let Some((kind, _)) = keys::describe(code) else {
            return;
        };
        match kind {
            Kind::Modifier(m) => {
                if !self.held.iter().any(|&(c, _)| c == code) {
                    self.held.push((code, m));
                }
                let caps = self.held_caps(now, Some(code));
                if let Some(g) = self.open_mut() {
                    if g.modifier_only() {
                        g.grow(caps, now);
                        return;
                    }
                }
                // A lone Shift is the start of a capital letter, which
                // shortcuts mode is not going to show.
                if rules.mode == Mode::Shortcuts && !self.command_modifier_held() {
                    return;
                }
                let id = self.push(caps, now);
                self.open = Some(id);
            }
            Kind::Mouse if !rules.show_mouse => {}
            _ => {
                if rules.mode == Mode::Shortcuts
                    && kind == Kind::Typing
                    && !self.command_modifier_held()
                {
                    return;
                }
                let Some(cap) = Cap::pressed(code, now) else {
                    return;
                };
                let mut caps = self.held_caps(now, None);
                caps.push(cap);
                if !self.held.is_empty() {
                    if let Some(g) = self.open_mut() {
                        if g.modifier_only() {
                            g.grow(caps, now);
                            return;
                        }
                        if g.same_stroke(&caps) {
                            g.repeat(code, now);
                            return;
                        }
                    }
                } else if let Some(g) = self.groups.last_mut() {
                    if g.leaving.is_none() && g.caps.len() == 1 && g.caps[0].code == code {
                        g.repeat(code, now);
                        return;
                    }
                }
                let id = self.push(caps, now);
                self.open = (!self.held.is_empty()).then_some(id);
            }
        }
    }

    pub fn release(&mut self, code: u16, now: Instant) {
        let modifier = keys::modifier(code);
        if let Some(i) = self.held.iter().position(|&(c, _)| c == code) {
            self.held.remove(i);
        }
        if self.held.is_empty() {
            self.open = None;
        }
        // Left and right Ctrl are one cap: it comes up when neither is down.
        let still_held = modifier.is_some_and(|m| self.held.iter().any(|&(_, h)| h == m));
        for g in &mut self.groups {
            let mut changed = false;
            for cap in &mut g.caps {
                let same_key = match modifier {
                    Some(m) => cap.modifier == Some(m) && !still_held,
                    None => cap.code == code,
                };
                if same_key && cap.down {
                    cap.down = false;
                    cap.changed = now;
                    changed = true;
                }
            }
            if changed && g.caps.iter().all(|c| !c.down) {
                g.released = Some(now);
            }
        }
    }

    /// Everything off the screen, as when the session locks. Which keys are
    /// physically down is kept, so a Ctrl held through it still combines.
    pub fn clear(&mut self) {
        self.groups.clear();
        self.open = None;
    }

    /// A device went away with keys down on it; nothing is held any more.
    pub fn forget_held(&mut self) {
        self.held.clear();
        self.open = None;
    }

    /// Start groups leaving whose time is up, and drop those that have left.
    pub fn tick(&mut self, now: Instant, rules: &Rules) {
        for g in &mut self.groups {
            if g.leaving.is_some() {
                continue;
            }
            if let Some(released) = g.released {
                if now >= released.max(g.bumped) + rules.hide_after {
                    g.leaving = Some(now);
                }
            }
        }
        let mut staying = self.groups.iter().filter(|g| g.leaving.is_none()).count();
        for g in &mut self.groups {
            if staying <= MAX_GROUPS {
                break;
            }
            if g.leaving.is_none() {
                g.leaving = Some(now);
                staying -= 1;
            }
        }
        self.groups
            .retain(|g| g.leaving.is_none_or(|t| now.duration_since(t) < EXIT));
        if let Some(id) = self.open {
            if !self
                .groups
                .iter()
                .any(|g| g.id == id && g.leaving.is_none())
            {
                self.open = None;
            }
        }
    }

    /// Whether anything is mid-movement, so another frame is wanted.
    pub fn animating(&self, now: Instant) -> bool {
        let within = |t: Instant, d: Duration| now.duration_since(t) < d;
        self.groups.iter().any(|g| {
            g.leaving.is_some()
                || within(g.born, ENTER)
                || within(g.bumped, POP)
                || g.caps
                    .iter()
                    .any(|c| within(c.changed, RELEASE) || within(c.pressed, RIPPLE))
        })
    }

    /// When the next group is due to start leaving, if any is waiting to.
    pub fn next_deadline(&self, rules: &Rules) -> Option<Instant> {
        self.groups
            .iter()
            .filter(|g| g.leaving.is_none())
            .filter_map(|g| g.released.map(|r| r.max(g.bumped) + rules.hide_after))
            .min()
    }

    fn push(&mut self, caps: Vec<Cap>, now: Instant) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.groups.push(Group {
            id,
            caps,
            count: 1,
            born: now,
            bumped: now,
            released: None,
            leaving: None,
        });
        id
    }

    fn open_mut(&mut self) -> Option<&mut Group> {
        let id = self.open?;
        self.groups
            .iter_mut()
            .find(|g| g.id == id && g.leaving.is_none())
    }

    fn command_modifier_held(&self) -> bool {
        self.held.iter().any(|&(_, m)| m != Modifier::Shift)
    }

    /// One cap per held modifier, in spelling order. `just_pressed` is the
    /// key going down now, which animates; the others were already down.
    fn held_caps(&self, now: Instant, just_pressed: Option<u16>) -> Vec<Cap> {
        let mut held = self.held.clone();
        held.sort_by_key(|&(_, m)| m);
        held.dedup_by_key(|&mut (_, m)| m);
        held.into_iter()
            .filter_map(|(code, m)| {
                let pressed_now = just_pressed
                    .and_then(keys::modifier)
                    .is_some_and(|j| j == m);
                if pressed_now {
                    Cap::pressed(just_pressed.unwrap_or(code), now)
                } else {
                    Cap::held(code, now)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::BTN_LEFT;

    const CTRL: u16 = 29;
    const RCTRL: u16 = 97;
    const SHIFT: u16 = 42;
    const SUPER: u16 = 125;
    const A: u16 = 30;
    const B: u16 = 48;
    const C: u16 = 46;
    const V: u16 = 47;
    const T: u16 = 20;
    const F5: u16 = 63;

    fn rules(mode: Mode) -> Rules {
        Rules {
            mode,
            show_mouse: true,
            hide_after: Duration::from_secs(2),
        }
    }

    fn labels(s: &Strokes) -> Vec<String> {
        s.groups
            .iter()
            .map(|g| {
                let caps: Vec<_> = g.caps.iter().map(|c| c.label).collect();
                let mut text = caps.join("+");
                if g.count > 1 {
                    text.push_str(&format!(" x{}", g.count));
                }
                text
            })
            .collect()
    }

    fn tap(s: &mut Strokes, code: u16, now: Instant, r: &Rules) {
        s.press(code, now, r);
        s.release(code, now);
    }

    #[test]
    fn modifiers_and_a_key_are_one_group() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        s.press(CTRL, now, &r);
        s.press(SHIFT, now, &r);
        s.press(T, now, &r);
        assert_eq!(labels(&s), ["Ctrl+Shift+T"]);
    }

    #[test]
    fn a_held_modifier_starts_a_group_per_key_and_counts_repeats() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        s.press(CTRL, now, &r);
        tap(&mut s, C, now, &r);
        tap(&mut s, V, now, &r);
        tap(&mut s, V, now, &r);
        assert_eq!(labels(&s), ["Ctrl+C", "Ctrl+V x2"]);
    }

    #[test]
    fn plain_keys_repeat_in_place_and_differ_beside() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        tap(&mut s, A, now, &r);
        tap(&mut s, A, now, &r);
        tap(&mut s, B, now, &r);
        assert_eq!(labels(&s), ["A x2", "B"]);
    }

    #[test]
    fn shortcuts_mode_hides_typing() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::Shortcuts));
        tap(&mut s, A, now, &r);
        s.press(SHIFT, now, &r);
        tap(&mut s, A, now, &r);
        s.release(SHIFT, now);
        assert!(s.groups.is_empty(), "{:?}", labels(&s));
        tap(&mut s, F5, now, &r);
        s.press(CTRL, now, &r);
        tap(&mut s, A, now, &r);
        assert_eq!(labels(&s), ["F5", "Ctrl+A"]);
    }

    #[test]
    fn shift_held_before_ctrl_still_joins_the_combination() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::Shortcuts));
        s.press(SHIFT, now, &r);
        s.press(CTRL, now, &r);
        s.press(T, now, &r);
        assert_eq!(labels(&s), ["Ctrl+Shift+T"]);
    }

    #[test]
    fn a_lone_modifier_shows() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        tap(&mut s, SUPER, now, &r);
        assert_eq!(labels(&s), ["Super"]);
        assert!(s.groups[0].released.is_some());
    }

    #[test]
    fn groups_leave_after_release_and_are_dropped_after_exit() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        s.press(A, now, &r);
        s.tick(now + Duration::from_secs(10), &r);
        assert!(s.groups[0].leaving.is_none(), "a held key never leaves");
        s.release(A, now + Duration::from_secs(10));
        let due = s.next_deadline(&r).unwrap();
        assert_eq!(due, now + Duration::from_secs(12));
        s.tick(due, &r);
        assert!(s.groups[0].leaving.is_some());
        s.tick(due + EXIT, &r);
        assert!(s.groups.is_empty());
    }

    #[test]
    fn the_oldest_group_makes_room() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        for code in [16, 17, 18, 19, 20, 21, 22] {
            tap(&mut s, code, now, &r);
        }
        s.tick(now, &r);
        let staying: Vec<_> = s.groups.iter().filter(|g| g.leaving.is_none()).collect();
        assert_eq!(staying.len(), MAX_GROUPS);
        assert!(s.groups[0].leaving.is_some());
    }

    #[test]
    fn mouse_buttons_follow_the_setting() {
        let now = Instant::now();
        let mut r = rules(Mode::All);
        r.show_mouse = false;
        let mut s = Strokes::default();
        tap(&mut s, BTN_LEFT, now, &r);
        assert!(s.groups.is_empty());
        r.show_mouse = true;
        s.press(CTRL, now, &r);
        tap(&mut s, BTN_LEFT, now, &r);
        assert_eq!(labels(&s), ["Ctrl+Click"]);
    }

    #[test]
    fn left_and_right_ctrl_are_one_cap() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        s.press(CTRL, now, &r);
        s.press(RCTRL, now, &r);
        assert_eq!(labels(&s), ["Ctrl"]);
        s.release(CTRL, now);
        assert!(s.groups[0].caps[0].down);
        s.release(RCTRL, now);
        assert!(!s.groups[0].caps[0].down);
        assert!(s.groups[0].released.is_some());
    }

    #[test]
    fn clearing_keeps_held_modifiers() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        s.press(CTRL, now, &r);
        s.clear();
        s.press(C, now, &r);
        assert_eq!(labels(&s), ["Ctrl+C"]);
        assert!(s.groups[0].caps[0].down);
    }

    #[test]
    fn nothing_animates_once_settled() {
        let (mut s, now, r) = (Strokes::default(), Instant::now(), rules(Mode::All));
        tap(&mut s, A, now, &r);
        assert!(s.animating(now));
        assert!(!s.animating(now + Duration::from_secs(1)));
    }
}
