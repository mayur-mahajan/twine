//! `.twinescript`: scripted input for headless runs.
//!
//! One command per line; `#` starts a comment (outside quotes); blank lines are ignored;
//! integers are decimal (coordinates and wheel steps may be negative).
//!
//! | Command | Effect |
//! |---------|--------|
//! | `wait <ms>` | advance the script time |
//! | `press <x> <y>` / `move <x> <y>` / `release` | pointer |
//! | `tap <x> <y>` | press, wait 50 ms, release |
//! | `drag <x0> <y0> <x1> <y1> <ms>` | press, linear moves every 16 ms, release (takes `ms`) |
//! | `key <Name>` | press + release of [`Key::from_name`] (`Enter`, `Up`, `a`, …) |
//! | `text "<chars>"` | press + release of each character (`\"` and `\\` escapes) |
//! | `wheel <diff>` | encoder rotation |
//! | `enc_press` / `enc_release` | encoder button |
//! | `shot <name>` | write `<out_dir>/<name>.png` |
//! | `hotkey <F1..F12>` | trigger a simulator hotkey |
//!
//! ```
//! use twine_sim::script::{parse, ScriptCmd};
//! let cmds = parse("wait 100 # settle\ntap 10 20\ntext \"hi\"\n").unwrap();
//! assert_eq!(cmds[0], ScriptCmd::Wait(100));
//! assert_eq!(cmds.len(), 3);
//! assert_eq!(parse("jump 1").unwrap_err().to_string(), "line 1: unknown command `jump`");
//! ```

use std::fmt;

use twine_core::Point;
use twine_hal::Key;

use crate::hotkeys::Hotkey;

/// Duration of the press in a `tap` command.
pub const TAP_MS: u64 = 50;
/// Interval of the intermediate moves of a `drag` command.
pub const DRAG_STEP_MS: u64 = 16;

/// One parsed script command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptCmd {
    /// `wait <ms>`.
    Wait(u64),
    /// `press <x> <y>`.
    Press(Point),
    /// `move <x> <y>`.
    Move(Point),
    /// `release`.
    Release,
    /// `tap <x> <y>`.
    Tap(Point),
    /// `drag <x0> <y0> <x1> <y1> <ms>`.
    Drag {
        /// Start point.
        from: Point,
        /// End point.
        to: Point,
        /// Duration in ms.
        ms: u64,
    },
    /// `key <Name>`.
    Key(Key),
    /// `text "<chars>"`.
    Text(String),
    /// `wheel <diff>`.
    Wheel(i16),
    /// `enc_press`.
    EncPress,
    /// `enc_release`.
    EncRelease,
    /// `shot <name>`.
    Shot(String),
    /// `hotkey <F1..F12>`.
    Hotkey(Hotkey),
}

/// A parse error with its 1-based line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    /// 1-based line number.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Splits a line into words, honouring one `"…"` string (with `\"` / `\\` escapes) and
/// stripping a `#` comment outside quotes. Quoted words are returned with a leading `"` marker.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            break;
        } else if c == '"' {
            chars.next();
            let mut s = String::from("\"");
            loop {
                match chars.next() {
                    None => return Err("unterminated string".into()),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(e @ ('"' | '\\')) => s.push(e),
                        Some(e) => return Err(format!("unknown escape `\\{e}` (only \\\" and \\\\)")),
                        None => return Err("unterminated string".into()),
                    },
                    Some(ch) => s.push(ch),
                }
            }
            words.push(s);
        } else {
            let mut w = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() || ch == '#' {
                    break;
                }
                w.push(ch);
                chars.next();
            }
            words.push(w);
        }
    }
    Ok(words)
}

fn int<T: std::str::FromStr>(w: &str, what: &str) -> Result<T, String> {
    if w.starts_with('"') {
        return Err(format!("expected {what}, got a string"));
    }
    w.parse().map_err(|_| format!("invalid {what} `{w}`"))
}

fn parse_line(words: &[String]) -> Result<ScriptCmd, String> {
    let (cmd, args) = words.split_first().ok_or("empty command")?;
    let expect = |n: usize| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!("`{cmd}` takes {n} argument(s), got {}", args.len()))
        }
    };
    let point = |i: usize| -> Result<Point, String> {
        Ok(Point::new(
            int(&args[i], "x coordinate")?,
            int(&args[i + 1], "y coordinate")?,
        ))
    };
    Ok(match cmd.as_str() {
        "wait" => {
            expect(1)?;
            ScriptCmd::Wait(int(&args[0], "milliseconds")?)
        }
        "press" => {
            expect(2)?;
            ScriptCmd::Press(point(0)?)
        }
        "move" => {
            expect(2)?;
            ScriptCmd::Move(point(0)?)
        }
        "release" => {
            expect(0)?;
            ScriptCmd::Release
        }
        "tap" => {
            expect(2)?;
            ScriptCmd::Tap(point(0)?)
        }
        "drag" => {
            expect(5)?;
            ScriptCmd::Drag {
                from: point(0)?,
                to: point(2)?,
                ms: int(&args[4], "milliseconds")?,
            }
        }
        "key" => {
            expect(1)?;
            let name = args[0].strip_prefix('"').unwrap_or(&args[0]);
            ScriptCmd::Key(Key::from_name(name).ok_or_else(|| format!("unknown key `{name}`"))?)
        }
        "text" => {
            expect(1)?;
            let s = args[0]
                .strip_prefix('"')
                .ok_or("`text` expects a quoted string")?;
            ScriptCmd::Text(s.to_string())
        }
        "wheel" => {
            expect(1)?;
            ScriptCmd::Wheel(int(&args[0], "wheel steps")?)
        }
        "enc_press" => {
            expect(0)?;
            ScriptCmd::EncPress
        }
        "enc_release" => {
            expect(0)?;
            ScriptCmd::EncRelease
        }
        "shot" => {
            expect(1)?;
            let name = &args[0];
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!(
                    "invalid shot name `{name}` (use letters, digits, _ and -)"
                ));
            }
            ScriptCmd::Shot(name.clone())
        }
        "hotkey" => {
            expect(1)?;
            ScriptCmd::Hotkey(
                Hotkey::from_name(&args[0]).ok_or_else(|| format!("unknown hotkey `{}`", args[0]))?,
            )
        }
        other => return Err(format!("unknown command `{other}`")),
    })
}

/// Parses a whole script.
pub fn parse(src: &str) -> Result<Vec<ScriptCmd>, ScriptError> {
    let mut cmds = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let err = |message: String| ScriptError { line: i + 1, message };
        let words = tokenize(line).map_err(err)?;
        if words.is_empty() {
            continue;
        }
        cmds.push(parse_line(&words).map_err(err)?);
    }
    Ok(cmds)
}

/// A timed input or output action derived from a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptAction {
    /// Pointer pressed at a point.
    Press(Point),
    /// Pointer moved.
    Move(Point),
    /// Pointer released.
    Release,
    /// Key pressed (`true`) or released.
    Key(Key, bool),
    /// Encoder rotation.
    Wheel(i16),
    /// Encoder button pressed (`true`) or released.
    EncButton(bool),
    /// Write a screenshot with this name.
    Shot(String),
    /// Trigger a hotkey.
    Hotkey(Hotkey),
}

impl ScriptAction {
    /// Whether the action produces output (applied after the frame at its time is rendered)
    /// rather than input (applied before).
    #[must_use]
    pub fn is_output(&self) -> bool {
        matches!(self, ScriptAction::Shot(_) | ScriptAction::Hotkey(_))
    }
}

/// Expands commands into actions with their script time in ms, in execution order.
///
/// ```
/// use twine_core::Point;
/// use twine_sim::script::{parse, schedule, ScriptAction};
/// let s = schedule(&parse("wait 20\ntap 1 2").unwrap());
/// assert_eq!(s, vec![(20, ScriptAction::Press(Point::new(1, 2))), (70, ScriptAction::Release)]);
/// ```
#[must_use]
pub fn schedule(cmds: &[ScriptCmd]) -> Vec<(u64, ScriptAction)> {
    let mut t = 0u64;
    let mut out = Vec::new();
    for c in cmds {
        match c {
            ScriptCmd::Wait(ms) => t = t.saturating_add(*ms),
            ScriptCmd::Press(p) => out.push((t, ScriptAction::Press(*p))),
            ScriptCmd::Move(p) => out.push((t, ScriptAction::Move(*p))),
            ScriptCmd::Release => out.push((t, ScriptAction::Release)),
            ScriptCmd::Tap(p) => {
                out.push((t, ScriptAction::Press(*p)));
                t += TAP_MS;
                out.push((t, ScriptAction::Release));
            }
            ScriptCmd::Drag { from, to, ms } => {
                out.push((t, ScriptAction::Press(*from)));
                let steps = (ms / DRAG_STEP_MS).max(1);
                for k in 1..=steps {
                    let lerp = |a: i32, b: i32| a + ((i64::from(b - a) * k as i64) / steps as i64) as i32;
                    let p = Point::new(lerp(from.x, to.x), lerp(from.y, to.y));
                    out.push((t + ms * k / steps, ScriptAction::Move(p)));
                }
                t = t.saturating_add(*ms);
                out.push((t, ScriptAction::Release));
            }
            ScriptCmd::Key(k) => {
                out.push((t, ScriptAction::Key(*k, true)));
                out.push((t, ScriptAction::Key(*k, false)));
            }
            ScriptCmd::Text(s) => {
                for ch in s.chars() {
                    out.push((t, ScriptAction::Key(Key::Char(ch), true)));
                    out.push((t, ScriptAction::Key(Key::Char(ch), false)));
                }
            }
            ScriptCmd::Wheel(d) => out.push((t, ScriptAction::Wheel(*d))),
            ScriptCmd::EncPress => out.push((t, ScriptAction::EncButton(true))),
            ScriptCmd::EncRelease => out.push((t, ScriptAction::EncButton(false))),
            ScriptCmd::Shot(n) => out.push((t, ScriptAction::Shot(n.clone()))),
            ScriptCmd::Hotkey(h) => out.push((t, ScriptAction::Hotkey(*h))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands() {
        let src = "\
# full example
wait 200
press 1 2
move -3 4   # comment after a command
release
tap 100 100

drag 10 10 200 150 300
key Enter
key a
text \"hi there\"
wheel -2
enc_press
enc_release
shot after_input
hotkey F9
";
        let cmds = parse(src).unwrap();
        assert_eq!(
            cmds,
            vec![
                ScriptCmd::Wait(200),
                ScriptCmd::Press(Point::new(1, 2)),
                ScriptCmd::Move(Point::new(-3, 4)),
                ScriptCmd::Release,
                ScriptCmd::Tap(Point::new(100, 100)),
                ScriptCmd::Drag {
                    from: Point::new(10, 10),
                    to: Point::new(200, 150),
                    ms: 300
                },
                ScriptCmd::Key(Key::Enter),
                ScriptCmd::Key(Key::Char('a')),
                ScriptCmd::Text("hi there".into()),
                ScriptCmd::Wheel(-2),
                ScriptCmd::EncPress,
                ScriptCmd::EncRelease,
                ScriptCmd::Shot("after_input".into()),
                ScriptCmd::Hotkey(Hotkey::Screenshot),
            ]
        );
    }

    #[test]
    fn reports_line_numbers() {
        let e = parse("wait 1\n\n# c\ntap 1\n").unwrap_err();
        assert_eq!(e.line, 4);
        assert!(e.message.contains("2 argument"), "{e}");
        assert_eq!(
            parse("wait x").unwrap_err().to_string(),
            "line 1: invalid milliseconds `x`"
        );
        assert_eq!(parse("text \"abc").unwrap_err().line, 1);
        assert!(parse("shot ../x").is_err());
        assert!(parse("hotkey F11").is_err());
    }

    #[test]
    fn text_escapes() {
        let cmds = parse(r#"text "a\"b\\c # not a comment""#).unwrap();
        assert_eq!(cmds, vec![ScriptCmd::Text("a\"b\\c # not a comment".into())]);
        assert!(parse(r#"text "bad \n""#).is_err());
        assert!(parse("text plain").is_err());
    }

    #[test]
    fn unknown_key_is_error() {
        let e = parse("key Enterr").unwrap_err();
        assert_eq!(e.to_string(), "line 1: unknown key `Enterr`");
    }

    #[test]
    fn drag_schedule_moves_linearly() {
        let s = schedule(&parse("drag 0 0 32 16 32").unwrap());
        assert_eq!(
            s,
            vec![
                (0, ScriptAction::Press(Point::new(0, 0))),
                (16, ScriptAction::Move(Point::new(16, 8))),
                (32, ScriptAction::Move(Point::new(32, 16))),
                (32, ScriptAction::Release),
            ]
        );
        let s = schedule(&parse("text \"ab\"\nkey Up").unwrap());
        assert_eq!(s.len(), 6);
        assert!(s.iter().all(|(t, a)| *t == 0 && !a.is_output()));
    }
}
