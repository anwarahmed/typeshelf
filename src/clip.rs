//! Copying to the system clipboard, without a clipboard crate: the platform's own tool,
//! else the terminal's clipboard escape (OSC 52).

use std::env;
use std::io::{Write, stdout};
use std::process::{Command, Stdio};

/// Puts `text` on the clipboard and returns how: the name of the tool that took it, or
/// "terminal" when it was handed to the terminal, which can't say whether it accepted it.
pub fn copy(text: &str) -> &'static str {
    for (tool, args) in tools() {
        if run(tool, args, text) {
            return tool;
        }
    }
    // Written between frames, like the cursor style; it draws nothing.
    let mut out = stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).and_then(|_| out.flush());
    "terminal"
}

/// The clipboard tools to try, in order. Over ssh the clipboard that matters is the one
/// on the user's side, which only the terminal can reach.
fn tools() -> Vec<(&'static str, &'static [&'static str])> {
    let set = |v: &str| env::var_os(v).is_some_and(|s| !s.is_empty());
    if set("SSH_CONNECTION") || set("SSH_TTY") {
        return vec![];
    }
    if cfg!(target_os = "macos") {
        return vec![("pbcopy", &[])];
    }
    let mut tools: Vec<(&'static str, &'static [&'static str])> = vec![];
    if set("WAYLAND_DISPLAY") {
        tools.push(("wl-copy", &[]));
    }
    if set("DISPLAY") {
        tools.push(("xclip", &["-selection", "clipboard"]));
        tools.push(("xsel", &["--clipboard", "--input"]));
    }
    tools
}

/// Runs a tool with `text` on its standard input. Its output is discarded, since the
/// terminal belongs to the app; wl-copy and xclip leave a process behind to serve the
/// clipboard, which is why stdin is closed before waiting.
fn run(tool: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(tool).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    let wrote = child.stdin.take().is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|s| s.success()) && wrote
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_like_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("é—".as_bytes()), "w6nigJQ=");
    }
}
