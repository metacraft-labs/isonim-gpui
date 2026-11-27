//! A two-stop conic gradient: the one `background-image` form the shim
//! paints, and the geometry of the sector it draws.
//!
//! # The supported subset
//!
//! CSS Images Level 4 §3.3 (conic gradients,
//! <https://www.w3.org/TR/css-images-4/#conic-gradients>) lets a colour run
//! round a point, starting at the top (0deg) and going clockwise. With two
//! colour stops that share one position, a "hard stop", the gradient is a
//! SECTOR of the first colour over the rest in the second:
//!
//! ```css
//! background-image: conic-gradient(<c1> 0 <p>%, <c2> <p>% 100%);
//! border-radius: 50%;
//! ```
//!
//! draws, in an element's disc, the sector from twelve o'clock clockwise to
//! `p`% of the turn in `c1`, and the rest of the disc in `c2`. That is the
//! whole subset: exactly two stops, the first from `0`, the second to
//! `100%`, both meeting at the same `p`%, no `from <angle>`, no
//! `at <position>`, no interpolation hint and no colour space. Any other
//! gradient is refused (`parse_two_stop_conic` answers `None`) and is left
//! unpainted, as the shim leaves every value it does not support.
//!
//! The parse and the geometry are pure functions, compiled in every build of
//! the shim and tested without a window (`cargo test`); `gpui_app` paints
//! with them (`with_conic_sector`).

/// A parsed two-stop conic gradient: `first` from the top clockwise to
/// `fraction` of the turn, `rest` after it. The colours are the CSS colour
/// spellings as written (the painter resolves them).
#[derive(Debug, Clone, PartialEq)]
pub struct TwoStopConic {
    pub first: String,
    pub rest: String,
    /// The hard stop's position, 0 to 1 of the turn.
    pub fraction: f32,
}

/// Splits a function's arguments at the commas that are not inside a nested
/// parenthesis (`rgb(1, 2, 3)` is one argument).
fn split_top_level(args: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, ch) in args.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    parts
}

/// Splits one colour stop, `<colour> <position> <position>`, into its colour
/// and its two positions. The colour may itself contain spaces inside a
/// function (`rgb(1 2 3)`), so the positions are taken from the end.
fn split_stop(stop: &str) -> Option<(&str, &str, &str)> {
    let stop = stop.trim();
    let last_space = stop.rfind(char::is_whitespace)?;
    let second = stop[last_space..].trim();
    let head = stop[..last_space].trim_end();
    let mid_space = head.rfind(char::is_whitespace)?;
    let first = head[mid_space..].trim();
    let colour = head[..mid_space].trim();
    if colour.is_empty() || colour.contains(')') && !colour.ends_with(')') {
        return None;
    }
    Some((colour, first, second))
}

/// A stop position: `0` or a percentage, as a fraction of the turn.
fn parse_position(s: &str) -> Option<f32> {
    if s == "0" {
        return Some(0.0);
    }
    let v: f32 = s.strip_suffix('%')?.parse().ok()?;
    if !v.is_finite() || !(0.0..=100.0).contains(&v) {
        return None;
    }
    Some(v / 100.0)
}

/// `conic-gradient(<c1> 0 <p>%, <c2> <p>% 100%)`, the supported subset, as a
/// `TwoStopConic`; `None` for any other value (see the module's header).
pub fn parse_two_stop_conic(value: &str) -> Option<TwoStopConic> {
    let value = value.trim();
    let args = value
        .strip_prefix("conic-gradient(")?
        .strip_suffix(')')?;
    let stops = split_top_level(args);
    if stops.len() != 2 {
        return None;
    }
    let (first, a0, a1) = split_stop(stops[0])?;
    let (rest, b0, b1) = split_stop(stops[1])?;
    // `from <angle>` and `at <position>` would make the first "colour"
    // start with those words; a colour never does.
    if first.starts_with("from ") || first.starts_with("at ") {
        return None;
    }
    let (a0, a1, b0, b1) = (
        parse_position(a0)?,
        parse_position(a1)?,
        parse_position(b0)?,
        parse_position(b1)?,
    );
    if a0 != 0.0 || b1 != 1.0 || a1 != b0 {
        return None;
    }
    Some(TwoStopConic {
        first: first.to_string(),
        rest: rest.to_string(),
        fraction: a1,
    })
}

/// The sector a `fraction` of the turn covers in the disc of centre
/// (`cx`, `cy`) and radius `r`, in the coordinates the painter draws in (y
/// grows DOWN the screen): it starts at twelve o'clock, `start`, and runs
/// CLOCKWISE to `end`. `large_arc` and `sweep` are the SVG arc flags of the
/// one arc from `start` to `end` (SVG 1.1 §8.3.8, the elliptical arc
/// command, <https://www.w3.org/TR/SVG11/paths.html#PathDataEllipticalArcCommands>):
/// `sweep` is true, the positive-angle direction, which on a y-down screen
/// is clockwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sector {
    pub start: (f32, f32),
    pub end: (f32, f32),
    pub large_arc: bool,
    pub sweep: bool,
}

/// What a `fraction` of the disc paints: nothing of the first colour, all of
/// it, or a sector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConicShape {
    /// No first colour: the disc is all `rest`.
    Empty,
    /// The disc is all `first`.
    Full,
    /// `first` covers this sector, `rest` the remainder.
    Sector(Sector),
}

/// The shape a `fraction` paints in the disc of centre (`cx`, `cy`) and
/// radius `r`. A fraction at or under 0 is empty and one at or over 1 full
/// (a sector of a whole turn has its start and end at one point, which an
/// arc cannot draw).
pub fn conic_shape(cx: f32, cy: f32, r: f32, fraction: f32) -> ConicShape {
    if !(fraction > 0.0) || r <= 0.0 {
        return ConicShape::Empty;
    }
    if fraction >= 1.0 {
        return ConicShape::Full;
    }
    let angle = fraction * std::f32::consts::TAU;
    ConicShape::Sector(Sector {
        start: (cx, cy - r),
        // Clockwise from the top on a y-down screen: x grows first.
        end: (cx + r * angle.sin(), cy - r * angle.cos()),
        large_arc: fraction > 0.5,
        sweep: true,
    })
}

/// Whether a box of `w` x `h` whose corners have `radius` is drawn as a
/// disc: the subset paints only on an element rounded into a circle
/// (`border-radius: 50%`, or a radius of at least half its shorter side).
pub fn is_round(w: f32, h: f32, radius: Option<&str>) -> bool {
    match radius.map(str::trim) {
        Some("50%") => true,
        Some(r) => r
            .strip_suffix("px")
            .unwrap_or(r)
            .parse::<f32>()
            .map(|v| v >= w.min(h) / 2.0)
            .unwrap_or(false),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conic(first: &str, rest: &str, fraction: f32) -> Option<TwoStopConic> {
        Some(TwoStopConic {
            first: first.into(),
            rest: rest.into(),
            fraction,
        })
    }

    #[test]
    fn the_supported_subset_parses() {
        assert_eq!(
            parse_two_stop_conic("conic-gradient(#ff0000 0 16%, #00ff00 16% 100%)"),
            conic("#ff0000", "#00ff00", 0.16)
        );
        assert_eq!(
            parse_two_stop_conic("  conic-gradient( #abc 0 0%, #def 0% 100% ) "),
            conic("#abc", "#def", 0.0)
        );
        assert_eq!(
            parse_two_stop_conic("conic-gradient(#111111 0 100%, #222222 100% 100%)"),
            conic("#111111", "#222222", 1.0)
        );
        assert_eq!(
            parse_two_stop_conic(
                "conic-gradient(rgb(1, 2, 3) 0 50%, rgba(4, 5, 6, 0.5) 50% 100%)"
            ),
            conic("rgb(1, 2, 3)", "rgba(4, 5, 6, 0.5)", 0.5)
        );
        assert_eq!(
            parse_two_stop_conic("conic-gradient(#000 0 12.5%, #fff 12.5% 100%)"),
            conic("#000", "#fff", 0.125)
        );
    }

    #[test]
    fn every_other_gradient_is_refused() {
        for refused in [
            // three stops, including three whose first two or whose first
            // and last would pass on their own
            "conic-gradient(#f00 0 10%, #0f0 10% 50%, #00f 50% 100%)",
            "conic-gradient(#f00 0 10%, #0f0 10% 100%, #00f 100% 100%)",
            "conic-gradient(#f00 0 10%, #0f0 10% 10%, #00f 10% 100%)",
            // one stop
            "conic-gradient(#f00 0 100%)",
            // the stops do not meet: a soft blend between them
            "conic-gradient(#f00 0 10%, #0f0 20% 100%)",
            // not from the top
            "conic-gradient(#f00 5% 10%, #0f0 10% 100%)",
            // not to the end
            "conic-gradient(#f00 0 10%, #0f0 10% 90%)",
            // a start angle or a centre
            "conic-gradient(from 90deg, #f00 0 10%, #0f0 10% 100%)",
            "conic-gradient(at 10% 10%, #f00 0 10%, #0f0 10% 100%)",
            // positions as angles, or out of range
            "conic-gradient(#f00 0 36deg, #0f0 36deg 100%)",
            "conic-gradient(#f00 0 120%, #0f0 120% 100%)",
            // single-position stops
            "conic-gradient(#f00, #0f0)",
            // other gradients and other images
            "linear-gradient(#f00 0 10%, #0f0 10% 100%)",
            "radial-gradient(#f00 0 10%, #0f0 10% 100%)",
            "repeating-conic-gradient(#f00 0 10%, #0f0 10% 100%)",
            "url(pie.svg)",
            "none",
            "",
        ] {
            assert_eq!(parse_two_stop_conic(refused), None, "{refused}");
        }
    }

    fn sector(fraction: f32) -> Sector {
        match conic_shape(10.0, 10.0, 10.0, fraction) {
            ConicShape::Sector(s) => s,
            other => panic!("{fraction}: {other:?}"),
        }
    }

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    #[test]
    fn the_sector_runs_clockwise_from_twelve_o_clock() {
        // A quarter ends at three o'clock: right of the centre, level with it.
        let q = sector(0.25);
        assert!(near(q.start, (10.0, 0.0)), "{q:?}");
        assert!(near(q.end, (20.0, 10.0)), "{q:?}");
        assert!(q.sweep && !q.large_arc, "{q:?}");
        // Half ends at six o'clock; three quarters at nine.
        assert!(near(sector(0.5).end, (10.0, 20.0)));
        assert!(near(sector(0.75).end, (0.0, 10.0)));
        assert!(sector(0.75).large_arc);
        // 16 % is in the upper RIGHT quadrant (clockwise), 84 % in the upper
        // left: a counter-clockwise sector would mirror both.
        let s16 = sector(0.16);
        assert!(s16.end.0 > 10.0 && s16.end.1 < 10.0, "{s16:?}");
        let s84 = sector(0.84);
        assert!(s84.end.0 < 10.0 && s84.end.1 < 10.0, "{s84:?}");
        assert!(!s16.large_arc && s84.large_arc);
        // 99 % ends just left of the top.
        let s99 = sector(0.99);
        assert!(s99.end.0 < 10.0 && s99.end.0 > 9.0, "{s99:?}");
    }

    #[test]
    fn nothing_and_everything_are_not_sectors() {
        assert_eq!(conic_shape(5.0, 5.0, 5.0, 0.0), ConicShape::Empty);
        assert_eq!(conic_shape(5.0, 5.0, 5.0, -0.5), ConicShape::Empty);
        assert_eq!(conic_shape(5.0, 5.0, 5.0, f32::NAN), ConicShape::Empty);
        assert_eq!(conic_shape(5.0, 5.0, 0.0, 0.5), ConicShape::Empty);
        assert_eq!(conic_shape(5.0, 5.0, 5.0, 1.0), ConicShape::Full);
        assert_eq!(conic_shape(5.0, 5.0, 5.0, 1.5), ConicShape::Full);
    }

    #[test]
    fn only_a_round_element_is_painted() {
        assert!(is_round(12.0, 12.0, Some("50%")));
        assert!(is_round(12.0, 12.0, Some("6px")));
        assert!(is_round(12.0, 12.0, Some("6")));
        assert!(is_round(10.0, 12.0, Some("5px")));
        assert!(!is_round(12.0, 12.0, Some("4px")));
        assert!(!is_round(12.0, 12.0, Some("25%")));
        assert!(!is_round(12.0, 12.0, None));
    }
}
