//! The examples of each command: the one source `--help` ("Examples:") and the usage
//! errors ([`crate::usage`]) both read, so what an error shows is what the help shows.
//!
//! An example is the arguments after `iohr`, never a string to split, so it needs no
//! parser to be checked: a test parses every one with the real command tree, and the `api`
//! ones against the routes of `spec/openapi.json`. A command with no entry of its own
//! falls back to its nearest parent that has one, then to the root's.

/// One runnable line: the arguments after `iohr`, and what it does in a few words.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Example {
    pub(crate) args: &'static [&'static str],
    pub(crate) what: &'static str,
}

const fn ex(args: &'static [&'static str], what: &'static str) -> Example {
    Example { args, what }
}

impl Example {
    /// The line as typed: `iohr`, then each argument quoted only when a shell would split or
    /// expand it.
    pub(crate) fn line(&self) -> String {
        let mut line = String::from("iohr");
        for a in self.args {
            line.push(' ');
            line.push_str(&quote(a));
        }
        line
    }
}

/// `text` as one shell word: as it is when it holds only safe characters, else in single
/// quotes (a single quote inside is closed, escaped and opened again).
pub(crate) fn quote(text: &str) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,#".contains(c));
    if safe {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

/// The commands that have examples, by path (`&[]` is the root, `&["token", "create"]` a
/// subcommand). Every visible top-level command is here; a test holds it to that.
pub(crate) const ALL: &[(&[&str], &[Example])] = &[
    (
        &[],
        &[
            ex(&["whoami"], "Who you are signed in as."),
            ex(&["api", "GET", "/v1/me"], "Call the API."),
            ex(
                &["ask", "how do I list my API tokens"],
                "Ask your own AI agent.",
            ),
        ],
    ),
    (
        &["login"],
        &[
            ex(&["login"], "Sign in, in a browser or with a code."),
            ex(
                &["login", "--device"],
                "Sign in with a code, for a machine with no browser.",
            ),
            ex(
                &["login", "--with-token"],
                "Add a profile from an API token on standard input.",
            ),
        ],
    ),
    (
        &["logout"],
        &[
            ex(&["logout"], "Forget the active profile on this machine."),
            ex(&["logout", "--profile", "work"], "Forget one profile."),
        ],
    ),
    (
        &["whoami"],
        &[
            ex(&["whoami"], "Subject, account, scopes and expiry."),
            ex(&["whoami", "--json"], "The same as JSON."),
        ],
    ),
    (
        &["profile"],
        &[
            ex(&["profile", "list"], "The profiles on this machine."),
            ex(&["profile", "use", "work"], "Make one the default."),
            ex(
                &["profile", "show"],
                "The active profile, never its secret.",
            ),
        ],
    ),
    (
        &["auth"],
        &[
            ex(
                &["auth", "token"],
                "The active profile's access token, alone on a line.",
            ),
            ex(
                &["auth", "token", "--format", "json"],
                "With its expiry, as JSON.",
            ),
        ],
    ),
    (
        &["token"],
        &[
            ex(&["token", "list"], "Your API tokens."),
            ex(
                &[
                    "token",
                    "create",
                    "--name",
                    "ci",
                    "--scope",
                    "decisions:read",
                    "--days",
                    "30",
                ],
                "A token for a program, printed once.",
            ),
            ex(
                &["token", "revoke", "TOKEN_ID"],
                "Revoke one (the id is in `token list`).",
            ),
        ],
    ),
    (
        &["token", "create"],
        &[
            ex(
                &[
                    "token",
                    "create",
                    "--name",
                    "ci",
                    "--scope",
                    "decisions:read",
                ],
                "A token with one scope.",
            ),
            ex(
                &[
                    "token",
                    "create",
                    "--name",
                    "ci",
                    "--scope",
                    "decisions:read",
                    "--days",
                    "30",
                ],
                "One that expires in 30 days.",
            ),
        ],
    ),
    (
        &["accounts"],
        &[
            ex(
                &["accounts", "list"],
                "The accounts your credential can see.",
            ),
            ex(&["accounts", "list", "--json"], "The same as JSON."),
        ],
    ),
    (
        &["domains"],
        &[
            ex(
                &["domains", "add", "acme.hr"],
                "Start proving a domain: prints the TXT record.",
            ),
            ex(
                &["domains", "verify", "acme.hr", "--wait"],
                "Check the record until it shows.",
            ),
            ex(&["domains", "list"], "The account's domains."),
        ],
    ),
    (
        &["connectors"],
        &[
            ex(
                &["connectors", "list"],
                "The apps a connection can be made from.",
            ),
            ex(
                &["connectors", "show", "incident-io"],
                "Its sign-in modes, actions and hosts.",
            ),
        ],
    ),
    (
        &["connections"],
        &[
            ex(&["connections", "list"], "The account's connections."),
            ex(&["connections", "add", "incident-io"], "Connect an app."),
            ex(
                &["connections", "test", "incident-io"],
                "Run its test request now.",
            ),
        ],
    ),
    (
        &["config"],
        &[
            ex(&["config", "get", "ext.registry"], "Print a value."),
            ex(
                &["config", "set", "ext.registry", "registry.example.com/iohr"],
                "Set one.",
            ),
            ex(&["config", "unset", "ext.registry"], "Back to the default."),
        ],
    ),
    (
        &["project"],
        &[
            ex(
                &["project", "init", "my-shop"],
                "A new project from the default template.",
            ),
            ex(
                &["project", "template", "list"],
                "The templates `project init` offers.",
            ),
        ],
    ),
    (
        &["atlas"],
        &[
            ex(&["atlas", "init"], "Add the atlas folder to this project."),
            ex(
                &["atlas", "new", "rfc", "Retry policy"],
                "A new RFC with the next free number.",
            ),
            ex(
                &["atlas", "sync", "--dry-run"],
                "What a sync would send, sending nothing.",
            ),
        ],
    ),
    (
        &["sync"],
        &[
            ex(&["sync"], "Share the project's record with your account."),
            ex(&["sync", "--json"], "The same, as JSON."),
        ],
    ),
    (
        &["ext"],
        &[
            ex(
                &["ext", "search", "capture"],
                "Search the extensions catalogue.",
            ),
            ex(
                &["ext", "show", "inorbit/capture"],
                "What one does and what it may do.",
            ),
            ex(
                &["ext", "install", "inorbit/capture"],
                "Verify and install it.",
            ),
        ],
    ),
    (
        &["sdk"],
        &[
            ex(&["sdk", "add"], "Add the published SDK to this project."),
            ex(
                &[
                    "sdk",
                    "generate",
                    "--lang",
                    "rust",
                    "--out",
                    "./inorbit-sdk",
                ],
                "An SDK cut to what your profile may call.",
            ),
            ex(&["sdk", "check"], "Fail when the cut has moved."),
        ],
    ),
    (
        &["api"],
        &[
            ex(&["api", "GET", "/v1/me"], "Your own identity."),
            ex(
                &["api", "--json", "GET", "/v1/webhooks/endpoints", "--all"],
                "Every page of a list, as JSON.",
            ),
            ex(
                &[
                    "api",
                    "POST",
                    "/v1/webhooks/endpoints",
                    "--input",
                    "body.json",
                ],
                "A JSON body from a file (`-` for standard input).",
            ),
            ex(
                &[
                    "api",
                    "POST",
                    "/v1/webhooks/endpoints",
                    "-f",
                    "url=https://example.com/hook",
                ],
                "Or body fields on the line.",
            ),
        ],
    ),
    (
        &["openapi"],
        &[
            ex(
                &["openapi", "pull"],
                "Save the document this credential sees.",
            ),
            ex(
                &["openapi", "pull", "--profile", "work"],
                "For another profile.",
            ),
        ],
    ),
    (
        &["lab"],
        &[
            ex(&["lab", "check"], "Check the documents here."),
            ex(&["lab", "check", "docs/rfcs"], "Check one folder."),
        ],
    ),
    (
        &["decisions"],
        &[
            ex(&["decisions", "list"], "The documents of your spaces."),
            ex(
                &["decisions", "show", "0065", "--text"],
                "One RFC with its text.",
            ),
            ex(
                &["decisions", "comment", "0065", "Looks good to me."],
                "Comment on it.",
            ),
        ],
    ),
    (
        &["ask"],
        &[
            ex(
                &["ask", "how do I roll back the last deploy"],
                "Ask your own AI agent about this folder.",
            ),
            ex(
                &["ask", "--dry-run", "why did the build fail"],
                "Print what it would run, run nothing.",
            ),
        ],
    ),
    (
        &["asks"],
        &[
            ex(&["asks", "list"], "The asks waiting for you."),
            ex(&["asks", "count"], "How many wait."),
            ex(&["asks", "show", "ASK_ID"], "One ask with its timeline."),
        ],
    ),
    (
        &["permissions"],
        &[
            ex(
                &["permissions", "list"],
                "The rules, and where each is kept.",
            ),
            ex(&["permissions", "allow", "Bash(ls:*)"], "Allow a command."),
            ex(
                &["permissions", "check", "Bash", "ls"],
                "What the rules say about one call.",
            ),
        ],
    ),
    (
        &["shell"],
        &[ex(
            &["shell", "hook", "zsh"],
            "Add `eval \"$(iohr shell hook zsh)\"` to ~/.zshrc.",
        )],
    ),
    (
        &["theme"],
        &[
            ex(&["theme", "list"], "The four themes and which one is on."),
            ex(&["theme", "show", "nord"], "The colours of one theme."),
            ex(
                &["theme", "install", "nord", "--prompt"],
                "Write the Terminal.app profile and turn the zsh prompt on.",
            ),
            ex(&["theme", "off"], "Turn the zsh prompt off; profiles stay."),
        ],
    ),
    (
        &["runner"],
        &[
            ex(
                &["runner", "status"],
                "Your runners and what this machine can run.",
            ),
            ex(
                &["runner", "register", "--name", "laptop"],
                "Register this machine.",
            ),
            ex(&["runner", "run"], "Run the loop until Ctrl-C."),
        ],
    ),
    (
        &["coordinator"],
        &[
            ex(&["coordinator", "jobs", "list"], "The coordinator's jobs."),
            ex(&["coordinator", "asks", "list"], "Its queued asks."),
            ex(&["coordinator", "brief", "JOB_ID"], "A job's brief."),
        ],
    ),
    (
        &["memory"],
        &[
            ex(
                &["memory", "remember", "We deploy on Fridays only"],
                "Keep a fact.",
            ),
            ex(
                &["memory", "recall", "deploys"],
                "Find what is known about a topic.",
            ),
            ex(&["memory", "list"], "Everything kept."),
        ],
    ),
    (
        &["tunnel"],
        &[
            ex(
                &["tunnel", "--host", "my-server", "--dry-run"],
                "Print the ssh command, open nothing.",
            ),
            ex(
                &["tunnel", "--host", "my-server", "--port", "8080"],
                "Open the pipe on port 8080.",
            ),
        ],
    ),
    (
        &["browser"],
        &[
            ex(
                &["browser", "install"],
                "Open the page that adds InOrbit Trails.",
            ),
            ex(
                &["browser", "install", "--print"],
                "Print the address instead.",
            ),
        ],
    ),
    (
        &["upgrade"],
        &[
            ex(
                &["upgrade", "--check"],
                "Only say whether a newer iohr exists.",
            ),
            ex(&["upgrade"], "Update iohr the way it was installed."),
        ],
    ),
    (
        &["completion"],
        &[
            ex(&["completion", "zsh"], "A completion script for zsh."),
            ex(
                &["completion", "bash"],
                "Or for bash, fish, elvish, powershell.",
            ),
        ],
    ),
];

/// The examples for the command at `path`: its own, else its nearest parent's, else the
/// root's. Returns the path they belong to with them.
pub(crate) fn for_path(path: &[String]) -> (&'static [&'static str], &'static [Example]) {
    for len in (0..=path.len()).rev() {
        let want = &path[..len];
        if let Some((p, list)) = ALL
            .iter()
            .find(|(p, _)| p.len() == want.len() && p.iter().zip(want).all(|(a, b)| a == b))
        {
            return (p, list);
        }
    }
    // `ALL` always holds the root; an empty answer keeps the caller total anyway.
    (&[], &[])
}

/// How the pieces of an examples block are painted.
pub(crate) struct Style<'a> {
    pub(crate) head: &'a dyn Fn(&str) -> String,
    pub(crate) command: &'a dyn Fn(&str) -> String,
    pub(crate) note: &'a dyn Fn(&str) -> String,
}

/// The block `--help` and a usage error share: a heading, then for each example a note line
/// and the command under it. The command is never split or padded, so a terminal's wrapping
/// cannot break it and it copies clean.
pub(crate) fn block(list: &[Example], style: &Style<'_>) -> String {
    use std::fmt::Write as _;
    let mut text = format!("{}\n", (style.head)("Examples:"));
    for e in list {
        let _ = writeln!(
            text,
            "  {}\n  {}",
            (style.note)(&format!("# {}", e.what)),
            (style.command)(&e.line()),
        );
    }
    text.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> clap::Command {
        crate::cli::command(false)
    }

    #[test]
    fn every_visible_top_level_command_has_examples_of_its_own() {
        let root = tree();
        let missing: Vec<_> = root
            .get_subcommands()
            .filter(|c| !c.is_hide_set() && c.get_name() != "help")
            .map(clap::Command::get_name)
            .filter(|name| {
                !ALL.iter()
                    .any(|(p, list)| p.len() == 1 && p[0] == *name && !list.is_empty())
            })
            .collect();
        assert!(missing.is_empty(), "no examples for: {missing:?}");
    }

    #[test]
    fn every_example_parses_with_the_real_command_tree() {
        let mut bad = Vec::new();
        for (path, list) in ALL {
            assert!(!list.is_empty(), "{path:?} has an empty list");
            for e in *list {
                let argv = std::iter::once("iohr").chain(e.args.iter().copied());
                if let Err(err) = tree().try_get_matches_from(argv) {
                    bad.push(format!("{}: {}", e.line(), err.render()));
                }
                // An example belongs to the command whose list it is in.
                assert!(
                    path.iter().all(|p| e.args.contains(p)),
                    "{} is listed under {path:?}",
                    e.line()
                );
                assert!(!e.what.is_empty() && e.what.ends_with('.'), "{}", e.line());
            }
        }
        assert!(
            bad.is_empty(),
            "examples that do not parse:\n{}",
            bad.join("\n")
        );
    }

    #[test]
    fn every_api_example_names_a_route_and_method_the_spec_has() {
        let spec: serde_json::Value =
            serde_json::from_str(include_str!("../../../../spec/openapi.json")).unwrap();
        for (_, list) in ALL {
            for e in *list {
                let Some(at) = e.args.iter().position(|a| *a == "api") else {
                    continue;
                };
                let rest = &e.args[at + 1..];
                let method = rest
                    .iter()
                    .find(|a| ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(a))
                    .unwrap();
                let path = rest.iter().find(|a| a.starts_with("/v1/")).unwrap();
                let route = &spec["paths"][*path];
                assert!(route.is_object(), "{}: {path} is not in the spec", e.line());
                assert!(
                    route[method.to_lowercase()].is_object(),
                    "{}: {path} has no {method}",
                    e.line()
                );
            }
        }
    }

    #[test]
    fn a_command_with_no_entry_falls_back_to_its_parent() {
        let path = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(for_path(&path(&["api"])).0, ["api"]);
        assert_eq!(for_path(&path(&["token", "create"])).0, ["token", "create"]);
        assert_eq!(for_path(&path(&["token", "revoke"])).0, ["token"]);
        assert_eq!(for_path(&path(&["nonsense"])).0.len(), 0);
    }

    #[test]
    fn quoting_keeps_a_word_one_word() {
        assert_eq!(quote("GET"), "GET");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }
}
