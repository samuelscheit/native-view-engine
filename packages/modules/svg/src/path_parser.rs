//! Bounded, panic-free subset SVG path `d="..."` parser (`M/m`, `L/l`, `H/h`, `V/v`, `Q/q`,
//! `T/t`, `C/c`, `S/s`, `Z/z`, implicit repeated commands, compact signs/decimals, scientific
//! notation).

use nve_abi::num::parse_f32;

use super::VectorPathCmd;

/// Maximum accepted SVG `d="..."` byte length (bounds adversarial inputs).
pub const MAX_SVG_PATH_INPUT_BYTES: usize = 256 * 1024;

/// Maximum emitted [`VectorPathCmd`] count per parsed path string.
pub const MAX_SVG_PATH_COMMANDS: usize = 16_384;

/// Maximum finite coordinate magnitude accepted from untrusted SVG path data.
pub(super) const MAX_SVG_COORD_ABS: f32 = 1.0e7;

/// Parses an SVG `d="..."` path data string into absolute [`VectorPathCmd`]s.
///
/// Supports `M/m`, `L/l`, `H/h`, `V/v`, `Q/q`, `T/t`, `C/c`, `S/s`, `Z/z`, implicit repeated
/// commands, compact negative/decimal separators (`M10-20`, `0.5.5`), and scientific notation.
/// Bounded by [`MAX_SVG_PATH_INPUT_BYTES`] and [`MAX_SVG_PATH_COMMANDS`]; malformed, huge,
/// or non-finite inputs stop cleanly without panicking.
pub fn parse_svg_path_data(d: &str) -> Vec<VectorPathCmd> {
    if d.is_empty() || d.len() > MAX_SVG_PATH_INPUT_BYTES {
        return Vec::new();
    }
    let bytes = d.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    let mut cmd: u8 = 0;
    let mut cur_x = 0.0f32;
    let mut cur_y = 0.0f32;
    let mut start_x = 0.0f32;
    let mut start_y = 0.0f32;
    let mut last_ctrl_quad: Option<(f32, f32)> = None;
    let mut last_ctrl_cubic: Option<(f32, f32)> = None;

    while i < bytes.len() && out.len() < MAX_SVG_PATH_COMMANDS {
        skip_separators(bytes, &mut i);
        if i >= bytes.len() {
            break;
        }
        let b = bytes[i];
        if b.is_ascii_alphabetic() {
            cmd = b;
            i += 1;
            if matches!(cmd, b'Z' | b'z') {
                out.push(VectorPathCmd::Close);
                cur_x = start_x;
                cur_y = start_y;
                last_ctrl_quad = None;
                last_ctrl_cubic = None;
                cmd = 0;
                continue;
            }
            if !matches!(
                cmd,
                b'M' | b'm'
                    | b'L'
                    | b'l'
                    | b'H'
                    | b'h'
                    | b'V'
                    | b'v'
                    | b'Q'
                    | b'q'
                    | b'T'
                    | b't'
                    | b'C'
                    | b'c'
                    | b'S'
                    | b's'
            ) {
                break;
            }
        } else if cmd == 0 {
            break;
        }

        let is_rel = cmd.is_ascii_lowercase();
        match cmd {
            b'M' | b'm' => {
                let (Some(x), Some(y)) =
                    (next_svg_number(bytes, &mut i), next_svg_number(bytes, &mut i))
                else {
                    break;
                };
                let (nx, ny) = if is_rel { (cur_x + x, cur_y + y) } else { (x, y) };
                if !coord_valid(nx) || !coord_valid(ny) {
                    break;
                }
                cur_x = nx;
                cur_y = ny;
                start_x = nx;
                start_y = ny;
                out.push(VectorPathCmd::MoveTo(nx, ny));
                last_ctrl_quad = None;
                last_ctrl_cubic = None;
                // Per SVG spec, subsequent coordinate pairs after M/m are implicit L/l.
                cmd = if is_rel { b'l' } else { b'L' };
            }
            b'L' | b'l' => {
                let (Some(x), Some(y)) =
                    (next_svg_number(bytes, &mut i), next_svg_number(bytes, &mut i))
                else {
                    break;
                };
                let (nx, ny) = if is_rel { (cur_x + x, cur_y + y) } else { (x, y) };
                if !coord_valid(nx) || !coord_valid(ny) {
                    break;
                }
                cur_x = nx;
                cur_y = ny;
                out.push(VectorPathCmd::LineTo(nx, ny));
                last_ctrl_quad = None;
                last_ctrl_cubic = None;
            }
            b'H' | b'h' => {
                let Some(x) = next_svg_number(bytes, &mut i) else {
                    break;
                };
                let nx = if is_rel { cur_x + x } else { x };
                if !coord_valid(nx) {
                    break;
                }
                cur_x = nx;
                out.push(VectorPathCmd::LineTo(cur_x, cur_y));
                last_ctrl_quad = None;
                last_ctrl_cubic = None;
            }
            b'V' | b'v' => {
                let Some(y) = next_svg_number(bytes, &mut i) else {
                    break;
                };
                let ny = if is_rel { cur_y + y } else { y };
                if !coord_valid(ny) {
                    break;
                }
                cur_y = ny;
                out.push(VectorPathCmd::LineTo(cur_x, cur_y));
                last_ctrl_quad = None;
                last_ctrl_cubic = None;
            }
            b'Q' | b'q' => {
                let (Some(cx), Some(cy), Some(x), Some(y)) = (
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                ) else {
                    break;
                };
                let (acx, acy, ax, ay) = if is_rel {
                    (cur_x + cx, cur_y + cy, cur_x + x, cur_y + y)
                } else {
                    (cx, cy, x, y)
                };
                if !coord_valid(acx) || !coord_valid(acy) || !coord_valid(ax) || !coord_valid(ay) {
                    break;
                }
                out.push(VectorPathCmd::QuadTo(acx, acy, ax, ay));
                last_ctrl_quad = Some((acx, acy));
                last_ctrl_cubic = None;
                cur_x = ax;
                cur_y = ay;
            }
            b'T' | b't' => {
                let (Some(x), Some(y)) =
                    (next_svg_number(bytes, &mut i), next_svg_number(bytes, &mut i))
                else {
                    break;
                };
                let (acx, acy) = match last_ctrl_quad {
                    Some((px, py)) => (2.0 * cur_x - px, 2.0 * cur_y - py),
                    None => (cur_x, cur_y),
                };
                let (ax, ay) = if is_rel { (cur_x + x, cur_y + y) } else { (x, y) };
                if !coord_valid(acx) || !coord_valid(acy) || !coord_valid(ax) || !coord_valid(ay) {
                    break;
                }
                out.push(VectorPathCmd::QuadTo(acx, acy, ax, ay));
                last_ctrl_quad = Some((acx, acy));
                last_ctrl_cubic = None;
                cur_x = ax;
                cur_y = ay;
            }
            b'C' | b'c' => {
                let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                ) else {
                    break;
                };
                let (ax1, ay1, ax2, ay2, ax, ay) = if is_rel {
                    (
                        cur_x + x1,
                        cur_y + y1,
                        cur_x + x2,
                        cur_y + y2,
                        cur_x + x,
                        cur_y + y,
                    )
                } else {
                    (x1, y1, x2, y2, x, y)
                };
                if !coord_valid(ax1)
                    || !coord_valid(ay1)
                    || !coord_valid(ax2)
                    || !coord_valid(ay2)
                    || !coord_valid(ax)
                    || !coord_valid(ay)
                {
                    break;
                }
                out.push(VectorPathCmd::CubicTo(ax1, ay1, ax2, ay2, ax, ay));
                last_ctrl_cubic = Some((ax2, ay2));
                last_ctrl_quad = None;
                cur_x = ax;
                cur_y = ay;
            }
            b'S' | b's' => {
                let (Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                    next_svg_number(bytes, &mut i),
                ) else {
                    break;
                };
                let (ax1, ay1) = match last_ctrl_cubic {
                    Some((px, py)) => (2.0 * cur_x - px, 2.0 * cur_y - py),
                    None => (cur_x, cur_y),
                };
                let (ax2, ay2, ax, ay) = if is_rel {
                    (cur_x + x2, cur_y + y2, cur_x + x, cur_y + y)
                } else {
                    (x2, y2, x, y)
                };
                if !coord_valid(ax1)
                    || !coord_valid(ay1)
                    || !coord_valid(ax2)
                    || !coord_valid(ay2)
                    || !coord_valid(ax)
                    || !coord_valid(ay)
                {
                    break;
                }
                out.push(VectorPathCmd::CubicTo(ax1, ay1, ax2, ay2, ax, ay));
                last_ctrl_cubic = Some((ax2, ay2));
                last_ctrl_quad = None;
                cur_x = ax;
                cur_y = ay;
            }
            _ => break,
        }
    }
    out
}

#[inline]
fn skip_separators(bytes: &[u8], i: &mut usize) {
    while *i < bytes.len() {
        let b = bytes[*i];
        if b.is_ascii_whitespace() || b == b',' {
            *i += 1;
        } else {
            break;
        }
    }
}

/// Scans one SVG path number token (handles compact signs like `10-20`, adjacent decimals like
/// `0.5.5`, leading dot `.5`, and scientific notation `1e-3`).
fn next_svg_number(bytes: &[u8], i: &mut usize) -> Option<f32> {
    skip_separators(bytes, i);
    if *i >= bytes.len() {
        return None;
    }
    let start = *i;
    if matches!(bytes[*i], b'+' | b'-') {
        *i += 1;
    }
    let mut saw_digit = false;
    while *i < bytes.len() && bytes[*i].is_ascii_digit() {
        saw_digit = true;
        *i += 1;
    }
    if *i < bytes.len() && bytes[*i] == b'.' {
        *i += 1;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            saw_digit = true;
            *i += 1;
        }
    }
    if !saw_digit {
        *i = start;
        return None;
    }
    if *i < bytes.len() && matches!(bytes[*i], b'e' | b'E') {
        let exp_start = *i;
        *i += 1;
        if *i < bytes.len() && matches!(bytes[*i], b'+' | b'-') {
            *i += 1;
        }
        let mut exp_digits = false;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            exp_digits = true;
            *i += 1;
        }
        if !exp_digits {
            *i = exp_start;
        }
    }
    let s = core::str::from_utf8(&bytes[start..*i]).ok()?;
    let val = parse_f32(s)?;
    coord_valid(val).then_some(val)
}

#[inline]
fn coord_valid(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_SVG_COORD_ABS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_path_commands_implicit_repeats_and_compact_numbers() {
        let cmds = parse_svg_path_data(
            "M0 0 10-5h5v10l-5 5H2V2Q4 0 8 2T16 2C17 0 19 0 20 2S23 4 24 2z",
        );
        assert_eq!(cmds[0], VectorPathCmd::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], VectorPathCmd::LineTo(10.0, -5.0));
        assert_eq!(cmds[2], VectorPathCmd::LineTo(15.0, -5.0));
        assert_eq!(cmds[3], VectorPathCmd::LineTo(15.0, 5.0));
        assert_eq!(cmds[4], VectorPathCmd::LineTo(10.0, 10.0));
        assert_eq!(cmds[5], VectorPathCmd::LineTo(2.0, 10.0));
        assert_eq!(cmds[6], VectorPathCmd::LineTo(2.0, 2.0));
        assert_eq!(cmds[7], VectorPathCmd::QuadTo(4.0, 0.0, 8.0, 2.0));
        assert_eq!(cmds[8], VectorPathCmd::QuadTo(12.0, 4.0, 16.0, 2.0));
        assert_eq!(cmds[9], VectorPathCmd::CubicTo(17.0, 0.0, 19.0, 0.0, 20.0, 2.0));
        assert_eq!(cmds[10], VectorPathCmd::CubicTo(21.0, 4.0, 23.0, 4.0, 24.0, 2.0));
        assert_eq!(cmds[11], VectorPathCmd::Close);

        let compact = parse_svg_path_data("m.5.5l1e1-2.5E0");
        assert_eq!(
            compact,
            vec![
                VectorPathCmd::MoveTo(0.5, 0.5),
                VectorPathCmd::LineTo(10.5, -2.0),
            ]
        );
    }

    #[test]
    fn malformed_and_adversarial_path_inputs_never_panic() {
        for bad in [
            "",
            "   ,,, ",
            "10 20",
            "M",
            "M 10",
            "M NaN 10",
            "M inf 10",
            "M 1e39 10",
            "M 0 0 L 1e9 0",
            "M 0 0 C 1 2 3",
            "M 0 0 Q 1 2",
            "X 10 20",
            "M 0 0 L 10 10 <script>alert(1)</script>",
        ] {
            let _ = parse_svg_path_data(bad);
        }
        let huge = "L 1 1 ".repeat(MAX_SVG_PATH_INPUT_BYTES / 5);
        assert!(parse_svg_path_data(&huge).is_empty());
    }
}
