//! `orcha settings`: every setting on a page in the browser, served from
//! 127.0.0.1 by a small background process. The page reads and writes the
//! same files the terminal Settings does, through the same edits, and asks
//! the running server to reload so a change shows at once.
//!
//! Only this machine can reach it, and only with the token in the URL: the
//! page sends it back in a header, which a page from another origin cannot.
//! A cookie lets the same browser reload the page; the API still wants the
//! header.
//!
//! `process` runs the background server, `http` reads and answers one
//! request, `routes` checks it and hands it to the settings model.

mod http;
mod process;
mod routes;

const USAGE: &str = "usage: orcha settings [stop]";

pub(crate) fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("open") => match open_quietly("") {
            Ok(Opened::Browser(url)) => {
                println!("Settings are open in your browser: {}", without_token(&url));
                0
            }
            Ok(Opened::Remote(url)) => {
                // the person opens it by hand on another machine: the whole
                // address, token and all, on their own terminal only
                println!("{}", remote_hint(&url));
                0
            }
            Err(message) => {
                eprintln!("{message}");
                // no browser here: the address is the only way in, so this
                // is the one place that prints it whole (never a log)
                if let Some(server) = process::running() {
                    eprintln!("Open it by hand: {}", server.url());
                }
                1
            }
        },
        Some("serve") => process::serve(),
        Some("stop") => {
            if process::stop() {
                println!("Settings page stopped.");
            } else {
                println!("The settings page is not running.");
            }
            0
        }
        Some("--help") | Some("-h") | Some("help") => {
            println!("{USAGE}");
            println!("  orcha settings       open every setting in the browser");
            println!("  orcha settings stop  stop the page's background server");
            0
        }
        Some(other) => {
            eprintln!("unknown settings command: {other}\n{USAGE}");
            2
        }
    }
}

/// Opens the page, starting its background server first when needed.
/// Blocking for up to a few seconds: call it off the input path.
pub(crate) enum Opened {
    Browser(String),
    /// Over SSH: the browser is on another machine, so only the address.
    Remote(String),
}

pub(crate) fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_TTY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// What to do with the address when orcha runs on another machine.
pub(crate) fn remote_hint(url: &str) -> String {
    let port = url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("PORT");
    format!(
        "orcha runs over SSH: forward the page first, then open it on your own machine.\n  ssh -L {port}:127.0.0.1:{port} <this host>\n  {url}"
    )
}

/// The page's address with its token blanked, for messages and logs.
fn without_token(url: &str) -> String {
    let (address, fragment) = match url.split_once('#') {
        Some((address, fragment)) => (address, Some(fragment)),
        None => (url, None),
    };
    let mut out = match address.split_once('?') {
        Some((path, query)) => {
            let query: Vec<&str> = query
                .split('&')
                .map(|pair| {
                    if pair.starts_with("t=") {
                        "t=<hidden>"
                    } else {
                        pair
                    }
                })
                .collect();
            format!("{path}?{}", query.join("&"))
        }
        None => address.to_owned(),
    };
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// `section` (say "background") scrolls the page to it. An error never
/// holds the token: callers log it and show it in notices.
pub(crate) fn open_quietly(section: &str) -> Result<Opened, String> {
    let server = process::running_or_start()?;
    let mut url = server.url();
    if !section.is_empty() {
        url = format!("{url}#{section}");
    }
    if over_ssh() {
        return Ok(Opened::Remote(url));
    }
    match crate::platform::open_url(&url) {
        Ok(Some(opener)) => process::reap(opener),
        Ok(None) => {}
        Err(err) => return Err(format!("could not open {}: {err}", without_token(&url))),
    }
    Ok(Opened::Browser(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_hint_names_the_port_to_forward() {
        let hint = remote_hint("http://127.0.0.1:64344/?t=abc");
        assert!(hint.contains("ssh -L 64344:127.0.0.1:64344"));
        assert!(hint.contains("http://127.0.0.1:64344/?t=abc"));
    }

    #[test]
    fn messages_show_the_address_without_its_token() {
        assert_eq!(
            without_token("http://127.0.0.1:64344/?t=abc#projects:/x?t=y"),
            "http://127.0.0.1:64344/?t=<hidden>#projects:/x?t=y"
        );
        assert_eq!(
            without_token("http://127.0.0.1:1/?a=1&t=abc"),
            "http://127.0.0.1:1/?a=1&t=<hidden>"
        );
        assert_eq!(without_token("http://127.0.0.1:1/"), "http://127.0.0.1:1/");
    }

    #[test]
    fn help_and_unknown_commands_change_nothing() {
        assert_eq!(run(&["--help".to_owned()]), 0);
        assert_eq!(run(&["fly".to_owned()]), 2);
    }
}
