use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::terminal::{self, Face};
use super::*;

fn cat() -> Catalogue {
    Catalogue::load().unwrap()
}

fn out() -> Out {
    Out { json: true }
}

fn args(name: &str) -> ThemeInstallArgs {
    ThemeInstallArgs {
        name: name.to_owned(),
        terminal: false,
        prompt: false,
        replace_prompt: false,
    }
}

/// Every file under `dir`, relative, sorted.
fn files(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, base: &Path, into: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, into);
            } else {
                // Join with '/' on every platform: Windows paths use '\\'.
                let rel = p.strip_prefix(base).unwrap();
                let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
                into.push(parts.join("/"));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

#[test]
fn the_catalogue_is_the_apps_four_themes_and_every_one_is_whole() {
    let c = cat();
    let ids: Vec<&str> = c.themes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["inorbit", "solarized-dark", "nord", "light"]);
    assert_eq!(c.default, "inorbit");
    for t in &c.themes {
        for k in COLOURS {
            assert!(Rgba::parse(&t.colors[k]).is_some(), "{} {k}", t.id);
        }
        assert_eq!(t.colors.len(), COLOURS.len(), "{}: no stray colours", t.id);
    }
    // The app's look: its accent is the InOrbit cursor, its font Geist Mono 12.5 with a bar.
    assert_eq!(c.get("inorbit").unwrap().colors["cursor"], "#ff9e1b");
    assert_eq!(c.font.family, "Geist Mono");
    assert!((c.font.size - 12.5).abs() < f64::EPSILON);
    assert_eq!(c.cursor.style, "bar");
}

#[test]
fn a_broken_catalogue_says_which_colour_and_a_missing_theme_lists_the_ids() {
    let bad = CATALOGUE.replacen("#0a0a0a", "0a0a0a", 1);
    let e = Catalogue::parse(&bad).unwrap_err().to_string();
    assert!(e.contains("inorbit: background is not a colour"), "{e}");
    let e = cat().find("dracula").unwrap_err().to_string();
    assert!(e.contains("inorbit, solarized-dark, nord, light"), "{e}");
    assert_eq!(cat().find("NORD").unwrap().id, "nord");
}

#[test]
fn hex_colours_parse_with_and_without_alpha_and_nothing_else() {
    assert_eq!(
        Rgba::parse("#ff9e1b"),
        Some(Rgba {
            r: 255,
            g: 158,
            b: 27,
            a: 255
        })
    );
    assert_eq!(Rgba::parse("#ffffff2e").unwrap().a, 0x2e);
    for odd in ["ff9e1b", "#fff", "#gg0000", "#ff9e1b2", "#é00000"] {
        assert_eq!(Rgba::parse(odd), None, "{odd}");
    }
    assert_eq!(Rgba::parse("#FF9E1B").unwrap().hex(), "#ff9e1b");
}

/// The NSRGB bytes of a colour archive: what Terminal reads back.
fn nsrgb(archive: &str) -> String {
    let at = archive.find("<key>NSRGB</key><data>").unwrap() + "<key>NSRGB</key><data>".len();
    let end = archive[at..].find("</data>").unwrap() + at;
    let bytes = STANDARD.decode(&archive[at..end]).unwrap();
    assert_eq!(bytes.last(), Some(&0), "NSRGB ends with a NUL");
    String::from_utf8(bytes[..bytes.len() - 1].to_vec()).unwrap()
}

#[test]
fn a_colour_archive_is_device_rgb_with_the_channels_as_fractions() {
    let a = terminal::color_archive(Rgba::parse("#ff9e1b").unwrap());
    assert!(
        a.contains("<key>NSColorSpace</key><integer>2</integer>"),
        "device RGB"
    );
    assert!(a.contains("<string>NSKeyedArchiver</string>"));
    assert_eq!(nsrgb(&a), "1.000000 0.619608 0.105882");
}

#[test]
fn the_selection_colour_is_blended_onto_the_background_and_opaque() {
    let bg = Rgba::parse("#0a0a0a").unwrap();
    let sel = terminal::composite(Rgba::parse("#ffffff2e").unwrap(), bg);
    // 255 * 46/255 + 10 * (1 - 46/255) = 46 + 8.196 = 54.196
    assert_eq!((sel.r, sel.g, sel.b, sel.a), (54, 54, 54, 255));
}

#[test]
fn the_profile_has_the_colours_the_font_and_the_cursor_of_the_theme() {
    let c = cat();
    let t = c.get("inorbit").unwrap();
    let face = Face {
        family: "Geist Mono".into(),
        postscript: "GeistMono-Regular",
    };
    let p = terminal::profile(&c, t, &face);
    for key in [
        "BackgroundColor",
        "TextColor",
        "TextBoldColor",
        "CursorColor",
        "SelectionColor",
        "ANSIBlackColor",
        "ANSIBrightWhiteColor",
        "Font",
    ] {
        assert!(p.contains(&format!("<key>{key}</key>")), "{key}");
    }
    assert_eq!(p.matches("<key>ANSI").count(), 16);
    assert!(
        p.contains("<key>CursorType</key>\n\t<integer>2</integer>"),
        "bar"
    );
    assert!(p.contains("<key>CursorBlink</key>\n\t<true/>"));
    assert!(p.contains("<key>name</key>\n\t<string>InOrbit</string>"));
    assert!(p.contains("<string>Window Settings</string>"));
    // The colour under CursorColor is the accent.
    let key = "<key>CursorColor</key>\n\t<data>";
    let at = p.find(key).unwrap() + key.len();
    let end = p[at..].find("</data>").unwrap() + at;
    let inner = String::from_utf8(STANDARD.decode(&p[at..end]).unwrap()).unwrap();
    assert_eq!(nsrgb(&inner), "1.000000 0.619608 0.105882");
    // The font archive names the face and the app's size.
    let key = "<key>Font</key>\n\t<data>";
    let at = p.find(key).unwrap() + key.len();
    let end = p[at..].find("</data>").unwrap() + at;
    let font = String::from_utf8(STANDARD.decode(&p[at..end]).unwrap()).unwrap();
    assert!(font.contains("<string>GeistMono-Regular</string>"));
    assert!(font.contains("<real>12.5</real>"));
}

#[test]
fn the_profile_names_are_inorbit_then_the_themes_label() {
    let c = cat();
    let names: Vec<String> = c.themes.iter().map(terminal::profile_name).collect();
    assert_eq!(
        names,
        [
            "InOrbit",
            "InOrbit Solarized dark",
            "InOrbit Nord",
            "InOrbit Light"
        ]
    );
}

#[test]
fn the_face_is_geist_mono_when_installed_then_sf_mono_then_menlo() {
    let c = cat();
    let with = terminal::choose_face(&c, &|f| f == "Geist Mono");
    assert_eq!(
        (with.family.as_str(), with.postscript),
        ("Geist Mono", "GeistMono-Regular")
    );
    let without = terminal::choose_face(&c, &|_| false);
    assert_eq!(
        (without.family.as_str(), without.postscript),
        ("SF Mono", "SFMonoTerminal-Regular")
    );
}

#[test]
fn the_prompt_snippet_has_the_themes_colours_and_no_placeholder_left() {
    let c = cat();
    let t = c.get("nord").unwrap();
    let s = render_prompt(t, PromptMode::Default);
    assert!(!s.contains("__IOHR_"), "a placeholder is left");
    for (k, role) in [
        ("blue", "_iohr_c_cwd"),
        ("magenta", "_iohr_c_git"),
        ("yellow", "_iohr_c_dirty"),
        ("cursor", "_iohr_c_ok"),
        ("red", "_iohr_c_bad"),
    ] {
        let want = format!("typeset -g {role}='%F{{{}}}'", t.rgba(k).hex());
        let loose = format!("{role}='%F{{{}}}'", t.rgba(k).hex());
        assert!(s.contains(&want) || s.contains(&loose), "{role} is {k}");
    }
    assert!(s.contains("_iohr_prompt_mode='default'"));
    assert!(render_prompt(t, PromptMode::Replace).contains("_iohr_prompt_mode='replace'"));
}

#[test]
fn install_writes_only_the_profile_and_the_state_and_remembers_the_theme() {
    let d = tempfile::tempdir().unwrap();
    install(&cat(), d.path(), &args("nord"), out()).unwrap();
    assert_eq!(files(d.path()), ["theme.json", "themes/nord.terminal"]);
    let state = read_state(d.path());
    assert_eq!(state.theme.as_deref(), Some("nord"));
    assert_eq!(state.prompt, PromptMode::Off, "the prompt is a flag");
    assert!(prompt_in(d.path()).is_none(), "off: the hook gets nothing");
    let profile = std::fs::read_to_string(profile_path(d.path(), "nord")).unwrap();
    assert!(profile.contains("<string>InOrbit Nord</string>"));
}

#[test]
fn the_prompt_flags_set_the_mode_and_off_clears_it_and_keeps_the_profile() {
    let d = tempfile::tempdir().unwrap();
    let mut a = args("light");
    a.prompt = true;
    install(&cat(), d.path(), &a, out()).unwrap();
    assert_eq!(read_state(d.path()).prompt, PromptMode::Default);
    assert!(
        prompt_in(d.path())
            .unwrap()
            .contains("_iohr_prompt_mode='default'")
    );
    a.replace_prompt = true;
    install(&cat(), d.path(), &a, out()).unwrap();
    assert_eq!(read_state(d.path()).prompt, PromptMode::Replace);
    assert!(
        prompt_in(d.path())
            .unwrap()
            .contains("_iohr_prompt_mode='replace'")
    );
    off(d.path(), out()).unwrap();
    let state = read_state(d.path());
    assert_eq!((state.theme, state.prompt), (None, PromptMode::Off));
    assert!(prompt_in(d.path()).is_none());
    assert!(
        profile_path(d.path(), "light").exists(),
        "the profile stays"
    );
}

#[test]
fn an_unknown_theme_writes_nothing() {
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(
        install(&cat(), d.path(), &args("dracula"), out()),
        Err(Error::Usage(_))
    ));
    assert_eq!(files(d.path()), Vec::<String>::new());
}

#[test]
fn a_damaged_state_file_is_off_not_an_error() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(state_path(d.path()), "{ not json").unwrap();
    assert!(prompt_in(d.path()).is_none());
    std::fs::write(
        state_path(d.path()),
        r#"{"theme":"gone","prompt":"replace"}"#,
    )
    .unwrap();
    assert!(
        prompt_in(d.path()).is_none(),
        "a theme that is no longer listed"
    );
}

#[test]
fn the_state_file_is_private() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        install(&cat(), d.path(), &args("nord"), out()).unwrap();
        let mode = std::fs::metadata(state_path(d.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn the_nearest_256_colour_is_the_cube_or_the_grey_ramp() {
    use crate::ui::Paint;
    let plain = Paint::plain();
    assert_eq!(
        plain.swatch((255, 158, 27), "#ff9e1b", true, false),
        "#ff9e1b"
    );
    let p = Paint::from_parts(true, false, None, Some("xterm-256color"));
    let s = p.swatch((255, 158, 27), "#ff9e1b", true, false);
    // #ff9e1b: red 255 -> level 5, green 158 -> 175 (3), blue 27 -> 0: 16 + 180 + 18 + 0 = 214.
    assert!(s.contains("48;5;214m"), "{s:?}");
    assert!(s.ends_with(" #ff9e1b"));
    let t = p.swatch((10, 10, 10), "#0a0a0a", false, true);
    assert!(t.contains("48;2;10;10;10m"), "{t:?}");
    assert!(!t.contains('#'));
}

#[test]
fn the_snippet_is_valid_zsh_once_rendered() {
    // zsh is not on every machine that runs the unit tests; the pty test needs it.
    let Ok(zsh) = std::process::Command::new("zsh").arg("--version").output() else {
        return;
    };
    assert!(zsh.status.success());
    let d = tempfile::tempdir().unwrap();
    let c = cat();
    for t in &c.themes {
        let file = d.path().join(format!("{}.zsh", t.id));
        std::fs::write(&file, render_prompt(t, PromptMode::Default)).unwrap();
        let r = std::process::Command::new("zsh")
            .arg("-n")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            r.status.success(),
            "{}: {}",
            t.id,
            String::from_utf8_lossy(&r.stderr)
        );
    }
}
