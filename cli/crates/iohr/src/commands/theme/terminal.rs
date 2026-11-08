//! The Terminal.app profile (`.terminal`) of a theme (RFC 0212).
//!
//! A `.terminal` file is an XML property list. Its colours and its font are `<data>` holding a
//! base64 `NSKeyedArchiver` archive of an `NSColor` / `NSFont`. Apple ships the file in the
//! binary archive form; the XML form of the same archive is read by the same
//! `NSKeyedUnarchiver` (checked in the tests' notes: `swift` decodes what is written here).
//!
//! Colours are written in `NSColorSpace` 2 (device RGB): the values reach the screen as
//! sRGB, so `#ff9e1b` is `#ff9e1b`. The calibrated space (1) would shift them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::{Catalogue, Rgba, Theme};

/// The keys of the sixteen ANSI colours, in Terminal's order, with the catalogue's names.
const ANSI: [(&str, &str); 16] = [
    ("ANSIBlackColor", "black"),
    ("ANSIRedColor", "red"),
    ("ANSIGreenColor", "green"),
    ("ANSIYellowColor", "yellow"),
    ("ANSIBlueColor", "blue"),
    ("ANSIMagentaColor", "magenta"),
    ("ANSICyanColor", "cyan"),
    ("ANSIWhiteColor", "white"),
    ("ANSIBrightBlackColor", "brightBlack"),
    ("ANSIBrightRedColor", "brightRed"),
    ("ANSIBrightGreenColor", "brightGreen"),
    ("ANSIBrightYellowColor", "brightYellow"),
    ("ANSIBrightBlueColor", "brightBlue"),
    ("ANSIBrightMagentaColor", "brightMagenta"),
    ("ANSIBrightCyanColor", "brightCyan"),
    ("ANSIBrightWhiteColor", "brightWhite"),
];

/// The font a profile names: Terminal.app takes one face, by PostScript name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Face {
    /// The family as a person knows it (`Geist Mono`).
    pub(super) family: String,
    /// The PostScript name Terminal looks the font up by.
    pub(super) postscript: &'static str,
}

/// The PostScript name of a family's regular face, for the families the app's stack names.
fn postscript(family: &str) -> Option<&'static str> {
    match family {
        "Geist Mono" => Some("GeistMono-Regular"),
        // The SF Mono that ships inside Terminal.app (the system one is not installable by name).
        "SF Mono" => Some("SFMonoTerminal-Regular"),
        "Menlo" => Some("Menlo-Regular"),
        _ => None,
    }
}

/// The first face of the app's stack (`font.family`, then `font.fallbacks`) that is installed.
/// SF Mono and Menlo ship with macOS, so `installed` is asked only about the rest; the last
/// resort is Menlo.
pub(super) fn choose_face(cat: &Catalogue, installed: &dyn Fn(&str) -> bool) -> Face {
    let stack = std::iter::once(&cat.font.family).chain(&cat.font.fallbacks);
    for family in stack {
        let Some(ps) = postscript(family) else {
            continue;
        };
        let system = matches!(family.as_str(), "SF Mono" | "Menlo");
        if system || installed(family) {
            return Face {
                family: family.clone(),
                postscript: ps,
            };
        }
    }
    Face {
        family: "Menlo".to_owned(),
        postscript: "Menlo-Regular",
    }
}

/// Terminal's `CursorType`: 0 block, 1 underline, 2 vertical bar (the shipped profiles use 0
/// and 1; the bar is Terminal's third shape).
fn cursor_type(style: &str) -> u8 {
    match style {
        "underline" => 1,
        "bar" => 2,
        _ => 0,
    }
}

/// `over` laid on `under`, opaque: Terminal's selection colour is drawn as it is, and the
/// app's is translucent white, so the blend is what the app shows.
pub(super) fn composite(over: Rgba, under: Rgba) -> Rgba {
    let a = f64::from(over.a) / 255.0;
    let mix = |o: u8, u: u8| -> u8 {
        let v = f64::from(o) * a + f64::from(u) * (1.0 - a);
        // `v` is between 0 and 255 by construction.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = v.round() as u8;
        v
    };
    Rgba {
        r: mix(over.r, under.r),
        g: mix(over.g, under.g),
        b: mix(over.b, under.b),
        a: 255,
    }
}

const PLIST_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n";

/// An `NSKeyedArchiver` archive with one root object (`object`, then its class).
fn archive(object: &str, class: &str, class_name: &str) -> String {
    format!(
        "{PLIST_HEAD}<dict>\n\
<key>$archiver</key><string>NSKeyedArchiver</string>\n\
<key>$objects</key><array>\n\
<string>$null</string>\n\
{object}\n\
<dict><key>$classes</key><array><string>{class_name}</string><string>NSObject</string></array>\
<key>$classname</key><string>{class}</string></dict>\n\
</array>\n\
<key>$top</key><dict><key>root</key><dict><key>CF$UID</key><integer>1</integer></dict></dict>\n\
<key>$version</key><integer>100000</integer>\n\
</dict>\n</plist>\n"
    )
}

/// The archive of one colour, as the bytes Terminal stores: device RGB, `"r g b\0"`.
pub(super) fn color_archive(c: Rgba) -> String {
    let f = |v: u8| format!("{:.6}", f64::from(v) / 255.0);
    let mut rgb = format!("{} {} {}", f(c.r), f(c.g), f(c.b)).into_bytes();
    rgb.push(0);
    let object = format!(
        "<dict><key>$class</key><dict><key>CF$UID</key><integer>2</integer></dict>\
<key>NSColorSpace</key><integer>2</integer><key>NSRGB</key><data>{}</data></dict>",
        STANDARD.encode(rgb)
    );
    archive(&object, "NSColor", "NSColor")
}

/// The archive of the font: its PostScript name and size.
pub(super) fn font_archive(face: &Face, size: f64) -> String {
    // The archive holds the name as a second object; the font dictionary points at it (UID 2)
    // and at its class (UID 3), as the profiles Apple ships do.
    format!(
        "{PLIST_HEAD}<dict>\n\
<key>$archiver</key><string>NSKeyedArchiver</string>\n\
<key>$objects</key><array>\n\
<string>$null</string>\n\
<dict><key>$class</key><dict><key>CF$UID</key><integer>3</integer></dict>\
<key>NSName</key><dict><key>CF$UID</key><integer>2</integer></dict>\
<key>NSSize</key><real>{size}</real><key>NSfFlags</key><integer>16</integer></dict>\n\
<string>{ps}</string>\n\
<dict><key>$classes</key><array><string>NSFont</string><string>NSObject</string></array>\
<key>$classname</key><string>NSFont</string></dict>\n\
</array>\n\
<key>$top</key><dict><key>root</key><dict><key>CF$UID</key><integer>1</integer></dict></dict>\n\
<key>$version</key><integer>100000</integer>\n\
</dict>\n</plist>\n",
        ps = face.postscript
    )
}

fn data(key: &str, archive: &str) -> String {
    format!(
        "\t<key>{key}</key>\n\t<data>{}</data>\n",
        STANDARD.encode(archive)
    )
}

/// The profile's name in Terminal's list: `InOrbit`, `InOrbit Nord`, `InOrbit Light`.
pub(super) fn profile_name(theme: &Theme) -> String {
    if theme.label.starts_with("InOrbit") {
        theme.label.clone()
    } else {
        format!("InOrbit {}", theme.label)
    }
}

/// The `.terminal` file of `theme`, in `face`, with the app's cursor.
pub(super) fn profile(cat: &Catalogue, theme: &Theme, face: &Face) -> String {
    let color = |k: &str| theme.rgba(k);
    let bg = color("background");
    let mut out = String::from(PLIST_HEAD);
    out.push_str("<dict>\n");
    let mut put = |key: &str, c: Rgba| out.push_str(&data(key, &color_archive(c)));
    put("BackgroundColor", bg);
    put("TextColor", color("foreground"));
    put("TextBoldColor", color("foreground"));
    put("CursorColor", color("cursor"));
    put(
        "SelectionColor",
        composite(color("selectionBackground"), bg),
    );
    for (key, name) in ANSI {
        put(key, color(name));
    }
    out.push_str(&data("Font", &font_archive(face, cat.font.size)));
    out.push_str(&format!(
        "\t<key>CursorBlink</key>\n\t<{}/>\n",
        cat.cursor.blink
    ));
    out.push_str(&format!(
        "\t<key>CursorType</key>\n\t<integer>{}</integer>\n",
        cursor_type(&cat.cursor.style)
    ));
    out.push_str("\t<key>FontAntialias</key>\n\t<true/>\n");
    out.push_str("\t<key>ProfileCurrentVersion</key>\n\t<real>2.09</real>\n");
    out.push_str("\t<key>name</key>\n");
    out.push_str(&format!("\t<string>{}</string>\n", profile_name(theme)));
    out.push_str("\t<key>type</key>\n\t<string>Window Settings</string>\n");
    out.push_str("</dict>\n</plist>\n");
    out
}
