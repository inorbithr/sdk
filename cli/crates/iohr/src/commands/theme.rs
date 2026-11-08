//! `iohr theme list|show|install|off`: the terminal look of the InOrbit app for the real
//! terminal (RFC 0212, PRD 0202, ADR 0225).
//!
//! The themes, the font and the cursor are the Mac app's, generated into `theme/themes.json`
//! by `tools/terminal-themes.py` (`mise run cli:themes`; `cli:themes:check` is the drift
//! check). From that one list this module makes a Terminal.app profile and a zsh prompt.
//!
//! Local files only; no sign-in, no network. Nothing here edits `~/.zshrc`, and nothing is
//! turned on that the person did not ask for: `install` writes the profile and remembers the
//! theme; the prompt, and opening the profile in Terminal.app, are flags.

mod terminal;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cli::{ThemeCommand, ThemeInstallArgs};
use crate::error::Error;
use crate::output::Out;
use crate::ui::{Paint, Ui};

/// The generated catalogue (`tools/terminal-themes.py`).
const CATALOGUE: &str = include_str!("theme/themes.json");
/// The prompt snippet, appended to the zsh hook when the prompt is on.
const ZSH_PROMPT: &str = include_str!("shell/prompt.zsh");
/// The colours every theme has, as the app names them.
const COLOURS: [&str; 21] = [
    "background",
    "foreground",
    "cursor",
    "cursorAccent",
    "selectionBackground",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
];

/// A colour as 8-bit channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rgba {
    pub(crate) r: u8,
    pub(crate) g: u8,
    pub(crate) b: u8,
    pub(crate) a: u8,
}

impl Rgba {
    /// `#rrggbb` or `#rrggbbaa`.
    pub(crate) fn parse(hex: &str) -> Option<Self> {
        let h = hex.strip_prefix('#')?;
        if !h.is_ascii() || !matches!(h.len(), 6 | 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if h.len() == 8 { byte(6)? } else { 255 },
        })
    }

    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Deserialize)]
struct Font {
    family: String,
    fallbacks: Vec<String>,
    size: f64,
}

#[derive(Debug, Deserialize)]
struct Cursor {
    style: String,
    blink: bool,
}

#[derive(Debug, Deserialize)]
struct Theme {
    id: String,
    label: String,
    dark: bool,
    colors: BTreeMap<String, String>,
}

impl Theme {
    /// A colour of the theme; the catalogue is checked whole when it is loaded.
    fn rgba(&self, key: &str) -> Rgba {
        self.colors
            .get(key)
            .and_then(|h| Rgba::parse(h))
            .unwrap_or(Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            })
    }

    fn mode(&self) -> &'static str {
        if self.dark { "dark" } else { "light" }
    }
}

/// The themes, the font and the cursor of the app.
#[derive(Debug, Deserialize)]
pub(crate) struct Catalogue {
    default: String,
    font: Font,
    cursor: Cursor,
    themes: Vec<Theme>,
}

impl Catalogue {
    pub(crate) fn load() -> Result<Self, Error> {
        Self::parse(CATALOGUE)
    }

    fn parse(text: &str) -> Result<Self, Error> {
        let broken = |why: String| {
            Error::Failed(format!(
                "the built-in theme list is broken ({why}); this is a bug in iohr"
            ))
        };
        let cat: Self = serde_json::from_str(text).map_err(|e| broken(e.to_string()))?;
        for t in &cat.themes {
            for k in COLOURS {
                if t.colors.get(k).and_then(|h| Rgba::parse(h)).is_none() {
                    return Err(broken(format!("{}: {k} is not a colour", t.id)));
                }
            }
        }
        if cat.get(&cat.default).is_none() {
            return Err(broken("the default theme is not listed".into()));
        }
        Ok(cat)
    }

    fn get(&self, id: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.id == id)
    }

    /// A theme by id, or a usage error that lists the ids.
    fn find(&self, id: &str) -> Result<&Theme, Error> {
        self.get(&id.to_ascii_lowercase()).ok_or_else(|| {
            let ids: Vec<&str> = self.themes.iter().map(|t| t.id.as_str()).collect();
            Error::Usage(format!(
                "no theme named {id}; the themes are {} (`iohr theme list`)",
                ids.join(", ")
            ))
        })
    }
}

/// What the prompt snippet does with a prompt that is not its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PromptMode {
    /// No prompt snippet.
    #[default]
    Off,
    /// Set the prompt unless the person's own is there.
    Default,
    /// Set the prompt always (`--replace-prompt`).
    Replace,
}

impl PromptMode {
    fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Default => "on, keeps a prompt you set yourself",
            Self::Replace => "on, replaces your prompt",
        }
    }

    fn snippet_word(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            _ => "default",
        }
    }
}

/// What `install` and `off` remember: `theme.json` in the terminal folder.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    theme: Option<String>,
    #[serde(default)]
    prompt: PromptMode,
}

/// `~/.inorbit/term` (`IOHR_TERM_DIR` moves it): where iohr keeps what the terminal needs, the
/// asks count the prompt reads among it.
fn term_dir() -> Result<PathBuf, Error> {
    if let Some(d) = std::env::var_os("IOHR_TERM_DIR").filter(|d| !d.is_empty()) {
        return Ok(d.into());
    }
    etcetera::home_dir()
        .map(|h| h.join(".inorbit").join("term"))
        .map_err(|_| Error::Failed("cannot find your home folder".to_owned()))
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("theme.json")
}

fn profile_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("themes").join(format!("{id}.terminal"))
}

/// The state; a missing or unreadable file is "nothing chosen".
fn read_state(dir: &Path) -> State {
    std::fs::read_to_string(state_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Writes a file whole or not at all (a temp file in the folder, then a rename), owner only.
fn write_file(path: &Path, text: &str) -> Result<(), Error> {
    let fail = |e: std::io::Error| Error::Failed(format!("cannot write {}: {e}", path.display()));
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(fail)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(fail)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

/// The zsh prompt snippet of `theme`, ready to be evaluated.
fn render_prompt(theme: &Theme, mode: PromptMode) -> String {
    let c = |k: &str| format!("%F{{{}}}", theme.rgba(k).hex());
    ZSH_PROMPT
        .replace("__IOHR_PROMPT_MODE__", mode.snippet_word())
        .replace("__IOHR_C_CWD__", &c("blue"))
        .replace("__IOHR_C_GIT__", &c("magenta"))
        .replace("__IOHR_C_DIRTY__", &c("yellow"))
        .replace("__IOHR_C_OK__", &c("cursor"))
        .replace("__IOHR_C_BAD__", &c("red"))
}

/// The prompt snippet `iohr shell hook zsh` appends, when `iohr theme install --prompt` turned
/// it on; `None` when it is off. Reading this never fails the hook: a missing or odd state
/// file, or a theme that is no longer listed, is "off".
pub(crate) fn hook_prompt() -> Option<String> {
    prompt_in(&term_dir().ok()?)
}

fn prompt_in(dir: &Path) -> Option<String> {
    let state = read_state(dir);
    if state.prompt == PromptMode::Off {
        return None;
    }
    let cat = Catalogue::load().ok()?;
    let theme = cat.get(state.theme.as_deref()?)?;
    Some(render_prompt(theme, state.prompt))
}

/// Whether a font family is installed for the person (`~/Library/Fonts`, `/Library/Fonts`).
fn font_installed(family: &str) -> bool {
    let stem = family.replace(' ', "");
    let mut dirs = vec![PathBuf::from("/Library/Fonts")];
    if let Ok(home) = etcetera::home_dir() {
        dirs.push(home.join("Library").join("Fonts"));
    }
    dirs.iter().any(|d| {
        std::fs::read_dir(d).is_ok_and(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&stem))
        })
    })
}

fn rgb(c: Rgba) -> (u8, u8, u8) {
    (c.r, c.g, c.b)
}

fn truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").ok().as_deref(),
        Some("truecolor" | "24bit")
    )
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub(crate) fn run(cmd: ThemeCommand, out: Out) -> Result<(), Error> {
    let cat = Catalogue::load()?;
    let dir = term_dir()?;
    match cmd {
        ThemeCommand::List => list(&cat, &dir, out),
        ThemeCommand::Show { name } => show(&cat, &dir, &name, out),
        ThemeCommand::Install(args) => install(&cat, &dir, &args, out),
        ThemeCommand::Off => off(&dir, out),
    }
}

fn list(cat: &Catalogue, dir: &Path, out: Out) -> Result<(), Error> {
    let state = read_state(dir);
    let active = state.theme.as_deref();
    if out.json {
        let rows: Vec<_> = cat
            .themes
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "label": t.label,
                    "mode": t.mode(),
                    "default": t.id == cat.default,
                    "active": active == Some(t.id.as_str()),
                })
            })
            .collect();
        Out::print_json(&json!({ "themes": rows, "prompt": state.prompt }));
        return Ok(());
    }
    let p = Paint::stdout(false);
    let rows: Vec<Vec<String>> = cat
        .themes
        .iter()
        .map(|t| {
            let on = active == Some(t.id.as_str());
            vec![
                p.name(&t.id),
                t.label.clone(),
                t.mode().to_owned(),
                if on { p.mark("*") } else { String::new() },
            ]
        })
        .collect();
    let head: Vec<String> = ["NAME", "LABEL", "MODE", "ON"]
        .iter()
        .map(|h| p.head(h))
        .collect();
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    Out::table(&head, &rows);
    if p.tty() {
        Out::note(&format!(
            "prompt: {}. `iohr theme show NAME` shows the colours; `iohr theme install NAME` installs one.",
            state.prompt.word()
        ));
    }
    Ok(())
}

fn show(cat: &Catalogue, dir: &Path, name: &str, out: Out) -> Result<(), Error> {
    let theme = cat.find(name)?;
    let face = terminal::choose_face(cat, &font_installed);
    let state = read_state(dir);
    if out.json {
        Out::print_json(&json!({
            "id": theme.id,
            "label": theme.label,
            "mode": theme.mode(),
            "active": state.theme.as_deref() == Some(theme.id.as_str()),
            "font": { "family": cat.font.family, "fallbacks": cat.font.fallbacks, "size": cat.font.size },
            "terminal_font": { "family": face.family, "postscript": face.postscript },
            "cursor": { "style": cat.cursor.style, "blink": cat.cursor.blink },
            "colors": theme.colors,
        }));
        return Ok(());
    }
    let p = Paint::stdout(false);
    let tc = truecolor();
    let swatch = |k: &str| p.swatch(rgb(theme.rgba(k)), &theme.colors[k], true, tc);
    let mut pairs: Vec<(&str, String)> = vec![
        ("theme", format!("{} ({})", theme.label, theme.id)),
        ("mode", theme.mode().to_owned()),
        (
            "font",
            format!(
                "{} {} (Terminal.app uses {})",
                cat.font.family, cat.font.size, face.family
            ),
        ),
        (
            "cursor",
            format!("{}, blink {}", cat.cursor.style, on_off(cat.cursor.blink)),
        ),
    ];
    for k in ["background", "foreground", "cursor", "selectionBackground"] {
        pairs.push((k, swatch(k)));
    }
    Out::pairs(&pairs);
    let row = |names: &[&str]| {
        names
            .iter()
            .map(|k| p.swatch(rgb(theme.rgba(k)), &theme.colors[*k], false, tc))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Out::pairs(&[
        (
            "ansi",
            row(&[
                "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
            ]),
        ),
        (
            "bright",
            row(&[
                "brightBlack",
                "brightRed",
                "brightGreen",
                "brightYellow",
                "brightBlue",
                "brightMagenta",
                "brightCyan",
                "brightWhite",
            ]),
        ),
    ]);
    Ok(())
}

fn install(cat: &Catalogue, dir: &Path, args: &ThemeInstallArgs, out: Out) -> Result<(), Error> {
    let theme = cat.find(&args.name)?;
    let prompt = if args.replace_prompt {
        PromptMode::Replace
    } else if args.prompt {
        PromptMode::Default
    } else {
        PromptMode::Off
    };
    if args.terminal && !cfg!(target_os = "macos") {
        return Err(Error::Usage(
            "--terminal opens the profile in Terminal.app, which only macOS has; the profile is still written without it"
                .to_owned(),
        ));
    }
    let face = terminal::choose_face(cat, &font_installed);
    let profile = profile_path(dir, &theme.id);
    write_file(&profile, &terminal::profile(cat, theme, &face))?;
    let state = State {
        theme: Some(theme.id.clone()),
        prompt,
    };
    let state_file = state_path(dir);
    write_file(
        &state_file,
        &(serde_json::to_string_pretty(&state).unwrap_or_default() + "\n"),
    )?;
    let opened = if args.terminal {
        open_in_terminal(&profile)?;
        true
    } else {
        false
    };
    let profile_name = terminal::profile_name(theme);
    let hook_on = zshrc_has_hook();
    let mut next: Vec<String> = Vec::new();
    if !opened {
        next.push(format!(
            "To use the Terminal.app profile, open it: open '{}'",
            profile.display()
        ));
    }
    if prompt != PromptMode::Off && !hook_on {
        next.push(
            "The prompt comes with the shell hook; add this to ~/.zshrc: eval \"$(iohr shell hook zsh)\""
                .to_owned(),
        );
    }
    if prompt == PromptMode::Off {
        next.push(format!(
            "To turn on the prompt: iohr theme install {} --prompt",
            theme.id
        ));
    }
    if out.json {
        Out::print_json(&json!({
            "theme": theme.id,
            "profile_name": profile_name,
            "wrote": [profile, state_file],
            "prompt": prompt,
            "opened_in_terminal": opened,
            "zshrc_has_hook": hook_on,
            "next": next,
        }));
        return Ok(());
    }
    let ui = Ui::detect(false);
    let lines = vec![
        format!("wrote {}", profile.display()),
        format!("wrote {}", state_file.display()),
    ];
    if ui.fancy() {
        ui.intro("iohr theme");
        for l in &lines {
            ui.done(l);
        }
        if opened {
            ui.done(&format!(
                "opened {profile_name} in Terminal.app (it is not made the default)"
            ));
        }
        ui.info(&format!("theme {}; prompt {}", theme.label, prompt.word()));
        ui.note("Next", &next.join("\n"));
    } else {
        for l in &lines {
            Out::raw(format!("{l}\n").as_bytes());
        }
        if opened {
            Out::raw(
                format!("opened {profile_name} in Terminal.app (it is not made the default)\n")
                    .as_bytes(),
            );
        }
        Out::raw(format!("theme {}; prompt {}\n", theme.label, prompt.word()).as_bytes());
        for l in &next {
            Out::raw(format!("{l}\n").as_bytes());
        }
    }
    Ok(())
}

/// `open FILE`: Terminal.app imports a `.terminal` profile and opens a window with it. It is
/// not made the default profile.
fn open_in_terminal(profile: &Path) -> Result<(), Error> {
    let status = std::process::Command::new("open")
        .arg(profile)
        .status()
        .map_err(|e| Error::Failed(format!("cannot run `open`: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed(format!(
            "`open {}` failed ({status})",
            profile.display()
        )))
    }
}

/// Whether `~/.zshrc` (or `$ZDOTDIR/.zshrc`) already evaluates the hook. A line that is not
/// a comment and names `shell hook zsh`.
fn zshrc_has_hook() -> bool {
    let rc = std::env::var_os("ZDOTDIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| etcetera::home_dir().ok())
        .map(|d| d.join(".zshrc"));
    rc.and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| {
            t.lines()
                .any(|l| !l.trim_start().starts_with('#') && l.contains("shell hook zsh"))
        })
}

fn off(dir: &Path, out: Out) -> Result<(), Error> {
    let before = read_state(dir);
    let state_file = state_path(dir);
    write_file(
        &state_file,
        &(serde_json::to_string_pretty(&State::default()).unwrap_or_default() + "\n"),
    )?;
    if out.json {
        Out::print_json(&json!({
            "was": before.theme,
            "prompt": PromptMode::Off,
            "wrote": [state_file],
            "kept": "profiles already imported into Terminal.app are not removed",
        }));
        return Ok(());
    }
    let ui = Ui::detect(false);
    let line = format!(
        "the prompt is off and no theme is chosen ({}). Open shells keep their prompt until they restart; a profile you imported into Terminal.app stays there.",
        state_file.display()
    );
    if ui.fancy() {
        ui.intro("iohr theme");
        ui.done(&line);
    } else {
        Out::raw(format!("{line}\n").as_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
