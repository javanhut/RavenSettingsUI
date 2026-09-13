//! The overlay's look: each group a glass panel of keycaps, laid out in a row.
//!
//! How things move, all on one clock:
//!
//! - a group arrives by rising and fading in (ease-out), and leaves by
//!   sinking and fading out (ease-in);
//! - a cap's face drops onto its lip while the key is down and springs back
//!   up when it is released, its face tinting towards the accent as it goes;
//! - each press sends a ring of the accent outward from the cap;
//! - a repeat count pops, and the other groups slide over when one leaves.
//!
//! With `motion` off (smooth animations switched off in Appearance) nothing
//! moves: groups only fade, and caps change colour without travelling.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::Position;
use crate::model::{self, Cap, Group, Strokes};
use crate::render::{Canvas, Rgba, Text};

const PANEL: Rgba = Rgba::rgb(16, 18, 24, 0.86);
const HAIRLINE: Rgba = Rgba::rgb(255, 255, 255, 0.10);
const FACE: Rgba = Rgba::rgb(54, 58, 72, 1.0);
const LIP: Rgba = Rgba::rgb(22, 24, 30, 1.0);
const SHINE: Rgba = Rgba::rgb(255, 255, 255, 0.12);
const LABEL: Rgba = Rgba::rgb(240, 242, 248, 1.0);
const MUTED: Rgba = Rgba::rgb(150, 156, 172, 1.0);

/// Every measurement, in logical pixels at medium size.
struct Metrics {
    cap_h: f32,
    lip: f32,
    font: f32,
    cap_pad: f32,
    cap_min_w: f32,
    cap_radius: f32,
    plus_w: f32,
    pad: f32,
    radius: f32,
    gap: f32,
    badge_font: f32,
    rise: f32,
    ripple: f32,
}

impl Metrics {
    fn at(s: f32) -> Metrics {
        Metrics {
            cap_h: 42.0 * s,
            lip: 5.0 * s,
            font: 16.0 * s,
            cap_pad: 13.0 * s,
            cap_min_w: 40.0 * s,
            cap_radius: 8.0 * s,
            plus_w: 18.0 * s,
            pad: 8.0 * s,
            radius: 14.0 * s,
            gap: 12.0 * s,
            badge_font: 15.0 * s,
            rise: 12.0 * s,
            ripple: 8.0 * s,
        }
    }

    fn group_h(&self) -> f32 {
        self.cap_h + 2.0 * self.pad
    }
}

/// The surface size a given size setting needs, in logical pixels: room for
/// a row of groups, and above and below it for the rise and the ripples.
pub fn surface_size(size: f32) -> (u32, u32) {
    let m = Metrics::at(size);
    let h = m.group_h() + 2.0 * (m.rise + m.ripple);
    ((1000.0 * size).ceil() as u32, h.ceil() as u32)
}

pub struct Style {
    /// Size setting times the buffer scale.
    pub scale: f32,
    pub accent: Rgba,
    pub motion: bool,
    pub position: Position,
}

/// Where each group is drawn, carried between frames so a group can slide
/// towards a new place instead of jumping to it.
#[derive(Default)]
pub struct Layout {
    x: HashMap<u64, f32>,
    last: Option<Instant>,
}

impl Layout {
    pub fn reset(&mut self) {
        self.x.clear();
        self.last = None;
    }
}

fn progress(since: Instant, now: Instant, d: Duration) -> f32 {
    (now.duration_since(since).as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn ease_in(t: f32) -> f32 {
    t * t
}

/// Draw every group. Returns whether anything is still sliding into place.
pub fn draw(
    canvas: &mut Canvas,
    text: &Text,
    strokes: &Strokes,
    style: &Style,
    layout: &mut Layout,
    now: Instant,
) -> bool {
    canvas.clear();
    let m = Metrics::at(style.scale);
    let width = canvas.width as f32;
    let top = ((canvas.height as f32 - m.group_h()) / 2.0).round();
    let dt = layout
        .last
        .map_or(0.0, |t| now.duration_since(t).as_secs_f32());
    layout.last = Some(now);

    // One row, oldest on the left. A leaving group keeps its place until it
    // has gone, so nothing slides under it while it fades; when the row is
    // wider than the surface the oldest are not drawn.
    let widths: Vec<f32> = strokes
        .groups
        .iter()
        .map(|g| group_width(g, text, &m))
        .collect();
    let mut placed: Vec<usize> = (0..strokes.groups.len()).collect();
    let row = |ids: &[usize]| {
        ids.iter().map(|&i| widths[i]).sum::<f32>() + m.gap * ids.len().saturating_sub(1) as f32
    };
    while placed.len() > 1 && row(&placed) > width {
        placed.remove(0);
    }
    let total = row(&placed);
    let mut target = match style.position {
        Position::BottomLeft => 0.0,
        Position::BottomRight => width - total,
        Position::BottomCentre | Position::TopCentre => ((width - total) / 2.0).round(),
    };

    let follow = 1.0 - (-dt * 16.0).exp();
    let mut sliding = false;
    // Just right of the previous group as drawn this frame: a new group
    // arrives there, beside its neighbour, rather than where the row will
    // be once it has finished sliding over to make room.
    let mut beside: Option<f32> = None;
    for &i in &placed {
        let g = &strokes.groups[i];
        let (gx, moving) = match (layout.x.get(&g.id), style.motion) {
            (Some(&current), true) => approach(current, target, follow),
            (None, true) => approach(beside.unwrap_or(target), target, 0.0),
            (_, false) => (target, false),
        };
        sliding |= moving;
        layout.x.insert(g.id, gx);
        beside = Some(gx + widths[i] + m.gap);
        target += widths[i] + m.gap;
        draw_group(canvas, text, g, gx, top, widths[i], &m, style, now);
    }
    layout
        .x
        .retain(|id, _| strokes.groups.iter().any(|g| g.id == *id));
    sliding
}

/// One frame of `current` closing on `target`; also whether it is still
/// short of it.
fn approach(current: f32, target: f32, follow: f32) -> (f32, bool) {
    let next = current + (target - current) * follow;
    if (target - next).abs() > 0.5 {
        (next, true)
    } else {
        (target, false)
    }
}

fn cap_width(cap: &Cap, text: &Text, m: &Metrics) -> f32 {
    (text.width(cap.label, m.font) + 2.0 * m.cap_pad)
        .max(m.cap_min_w)
        .round()
}

fn badge(g: &Group) -> Option<String> {
    (g.count > 1).then(|| format!("×{}", g.count))
}

fn group_width(g: &Group, text: &Text, m: &Metrics) -> f32 {
    let caps: f32 = g.caps.iter().map(|c| cap_width(c, text, m)).sum();
    let plus = m.plus_w * g.caps.len().saturating_sub(1) as f32;
    let count = badge(g).map_or(0.0, |b| m.pad + text.width(&b, m.badge_font) + m.pad * 0.5);
    2.0 * m.pad + caps + plus + count
}

#[allow(clippy::too_many_arguments)]
fn draw_group(
    canvas: &mut Canvas,
    text: &Text,
    g: &Group,
    x: f32,
    top: f32,
    w: f32,
    m: &Metrics,
    style: &Style,
    now: Instant,
) {
    let enter = ease_out(progress(g.born, now, model::ENTER));
    let exit = g
        .leaving
        .map_or(0.0, |t| ease_in(progress(t, now, model::EXIT)));
    let alpha = enter * (1.0 - exit);
    if alpha <= 0.0 {
        return;
    }
    let rise = if style.motion {
        (1.0 - enter) * m.rise + exit * m.rise * 0.7
    } else {
        0.0
    };
    // Up from the bottom of the screen, down from the top.
    let top = match style.position {
        Position::TopCentre => top - rise,
        _ => top + rise,
    };

    canvas.fill_rounded(x, top, w, m.group_h(), m.radius, PANEL.alpha(alpha));
    canvas.stroke_rounded(
        x,
        top,
        w,
        m.group_h(),
        m.radius,
        style.scale.max(1.0),
        HAIRLINE.alpha(alpha),
    );

    let mut cx = x + m.pad;
    let cap_top = top + m.pad;
    for (i, cap) in g.caps.iter().enumerate() {
        if i > 0 {
            let pw = text.width("+", m.font);
            text.draw(
                canvas,
                "+",
                cx + (m.plus_w - pw) / 2.0,
                cap_top + (m.cap_h - m.lip) / 2.0,
                m.font,
                MUTED.alpha(alpha),
            );
            cx += m.plus_w;
        }
        let cw = cap_width(cap, text, m);
        draw_cap(canvas, text, cap, cx, cap_top, cw, m, style, alpha, now);
        cx += cw;
    }

    if let Some(b) = badge(g) {
        let pop = ease_out(progress(g.bumped, now, model::POP));
        let px = if style.motion {
            m.badge_font * (1.0 + 0.4 * (1.0 - pop))
        } else {
            m.badge_font
        };
        let bw = text.width(&b, px);
        let slot = text.width(&b, m.badge_font);
        text.draw(
            canvas,
            &b,
            cx + m.pad + (slot - bw) / 2.0,
            cap_top + (m.cap_h - m.lip) / 2.0,
            px,
            style.accent.mix(LABEL, 0.25).alpha(alpha),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_cap(
    canvas: &mut Canvas,
    text: &Text,
    cap: &Cap,
    x: f32,
    top: f32,
    w: f32,
    m: &Metrics,
    style: &Style,
    alpha: f32,
    now: Instant,
) {
    // 0 is up, 1 is all the way down.
    let down = match (cap.down, style.motion) {
        (true, true) => ease_out(progress(cap.changed, now, model::PRESS)),
        (false, true) => 1.0 - ease_out(progress(cap.changed, now, model::RELEASE)),
        (true, false) => 1.0,
        (false, false) => 0.0,
    };
    let face_h = m.cap_h - m.lip;
    let face_top = top + m.lip * 0.8 * down;
    let pressed_face = FACE.mix(style.accent, 0.42);

    // The lip is the cap's side, showing below the face until the face is
    // pushed down over it.
    canvas.fill_rounded(x, top + m.lip, w, face_h, m.cap_radius, LIP.alpha(alpha));
    canvas.fill_rounded(
        x,
        face_top,
        w,
        face_h,
        m.cap_radius,
        FACE.mix(pressed_face, down).alpha(alpha),
    );
    let line = style.scale.max(1.0);
    canvas.stroke_rounded(
        x,
        face_top,
        w,
        face_h,
        m.cap_radius,
        line,
        SHINE.alpha(alpha * (1.0 - down * 0.6)),
    );
    if down > 0.0 {
        canvas.stroke_rounded(
            x,
            face_top,
            w,
            face_h,
            m.cap_radius,
            1.5 * line,
            style.accent.alpha(alpha * down),
        );
    }

    if style.motion {
        let t = progress(cap.pressed, now, model::RIPPLE);
        if t < 1.0 {
            let grow = ease_out(t) * m.ripple;
            canvas.stroke_rounded(
                x - grow,
                face_top - grow,
                w + 2.0 * grow,
                face_h + 2.0 * grow,
                m.cap_radius + grow,
                2.0 * line,
                // Squared, so the ring is gone before it can read as an
                // outline around the cap.
                style.accent.alpha(alpha * (1.0 - t).powi(2) * 0.55),
            );
        }
    }

    let tw = text.width(cap.label, m.font);
    text.draw(
        canvas,
        cap.label,
        x + ((w - tw) / 2.0).round(),
        face_top + face_h / 2.0,
        m.font,
        LABEL.alpha(alpha),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Rules};

    /// Development aid, like raven-settings' snapshot mode: with
    /// `RAVEN_KEYCAST_FRAMES=<dir>`, `cargo test --release -p raven-keycast --
    /// --ignored frames` plays a sequence of keys on the animation clock at
    /// 60 Hz and saves a few frames over a grey backdrop as raw RGB, so the
    /// overlay can be looked at without a compositor or a keyboard.
    #[test]
    #[ignore]
    fn frames() {
        let Some(dir) = std::env::var_os("RAVEN_KEYCAST_FRAMES") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = Text::load().unwrap();
        let rules = Rules {
            mode: Mode::All,
            show_mouse: true,
            hide_after: Duration::from_secs(2),
        };
        let style = Style {
            scale: 2.0,
            accent: Rgba::parse_hex("#7AA2F7").unwrap(),
            motion: true,
            position: Position::BottomCentre,
        };
        // Esc; A three times; Ctrl, Shift, T held, then let go.
        let events: &[(u64, u16, bool)] = &[
            (0, 1, true),
            (80, 1, false),
            (300, 30, true),
            (360, 30, false),
            (420, 30, true),
            (480, 30, false),
            (540, 30, true),
            (600, 30, false),
            (1000, 29, true),
            (1040, 42, true),
            (1100, 20, true),
            (1600, 20, false),
            (1650, 42, false),
            (1700, 29, false),
        ];
        let shots: &[(u64, &str)] = &[
            (1024, "ctrl-arriving"),
            (1120, "t-pressed-ripple"),
            (1408, "combo-held"),
            (1712, "released-springing-up"),
            (2192, "esc-leaving"),
            (2416, "row-sliding-over"),
        ];
        let (w, h) = surface_size(1.0);
        let (pw, ph) = (w * 2, h * 2);
        let mut buf = vec![0u8; (pw * ph * 4) as usize];
        let t0 = Instant::now();
        let mut strokes = Strokes::default();
        let mut layout = Layout::default();
        let mut next_event = 0;
        for (n, ms) in (0..=2416u64).step_by(16).enumerate() {
            let now = t0 + Duration::from_millis(ms);
            while next_event < events.len() && events[next_event].0 <= ms {
                let (at, code, down) = events[next_event];
                let at = t0 + Duration::from_millis(at);
                if down {
                    strokes.press(code, at, &rules);
                } else {
                    strokes.release(code, at);
                }
                next_event += 1;
            }
            strokes.tick(now, &rules);
            let mut canvas = Canvas {
                buf: &mut buf,
                width: pw,
                height: ph,
            };
            draw(&mut canvas, &text, &strokes, &style, &mut layout, now);
            let Some(&(_, label)) = shots.iter().find(|s| s.0 == ms) else {
                continue;
            };
            let backdrop = [58.0, 64.0, 80.0];
            let mut rgb = Vec::with_capacity((pw * ph * 3) as usize);
            for px in buf.as_chunks::<4>().0 {
                let a = px[3] as f32 / 255.0;
                for (c, bg) in [px[2], px[1], px[0]].into_iter().zip(backdrop) {
                    rgb.push((c as f32 + bg * (1.0 - a)).round().min(255.0) as u8);
                }
            }
            std::fs::write(dir.join(format!("{n:03}-{label}.rgb")), rgb).unwrap();
        }
    }
}
