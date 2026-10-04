//! Color themes. "terminal" uses only the terminal's own ANSI palette, so it follows
//! whatever theme the terminal has; the others are truecolor.

use ratatui::style::Color;

#[derive(Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    /// `None` leaves the terminal's own background.
    pub bg: Option<Color>,
    /// Text that has been typed, and ordinary UI text.
    pub fg: Color,
    /// Text still to type, hints, borders.
    pub dim: Color,
    pub accent: Color,
    pub error: Color,
    /// Typed correctly after a mistake.
    pub fixed: Color,
    pub good: Color,
    pub sel_bg: Color,
    pub sel_fg: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn truecolor(name: &'static str, c: [u32; 8]) -> Theme {
    Theme {
        name,
        bg: Some(rgb(c[0])),
        fg: rgb(c[1]),
        dim: rgb(c[2]),
        accent: rgb(c[3]),
        error: rgb(c[4]),
        fixed: rgb(c[5]),
        good: rgb(c[6]),
        sel_bg: rgb(c[7]),
        sel_fg: rgb(c[1]),
    }
}

pub const THEMES: [Theme; 8] = [
    Theme {
        name: "terminal",
        bg: None,
        fg: Color::Reset,
        dim: Color::DarkGray,
        accent: Color::Cyan,
        error: Color::Red,
        fixed: Color::Yellow,
        good: Color::Green,
        sel_bg: Color::DarkGray,
        sel_fg: Color::White,
    },
    //                         bg        fg        dim       accent    error     fixed     good      sel_bg
    truecolor("tokyo night", [0x1a1b26, 0xc0caf5, 0x565f89, 0x7aa2f7, 0xf7768e, 0xe0af68, 0x9ece6a, 0x283457]),
    truecolor("catppuccin", [0x1e1e2e, 0xcdd6f4, 0x6c7086, 0xcba6f7, 0xf38ba8, 0xf9e2af, 0xa6e3a1, 0x313244]),
    truecolor("gruvbox", [0x282828, 0xebdbb2, 0x7c6f64, 0xfabd2f, 0xfb4934, 0xfe8019, 0xb8bb26, 0x3c3836]),
    truecolor("nord", [0x2e3440, 0xeceff4, 0x616e88, 0x88c0d0, 0xbf616a, 0xebcb8b, 0xa3be8c, 0x3b4252]),
    truecolor("rose pine", [0x191724, 0xe0def4, 0x6e6a86, 0xc4a7e7, 0xeb6f92, 0xf6c177, 0x9ccfd8, 0x26233a]),
    truecolor("everforest", [0x2d353b, 0xd3c6aa, 0x7a8478, 0xa7c080, 0xe67e80, 0xdbbc7f, 0x83c092, 0x3d484d]),
    truecolor("paper", [0xf5f0e6, 0x2b2a27, 0xa39e93, 0x3b6ea5, 0xc0392b, 0xb5760a, 0x4d7c2a, 0xe4dccb]),
];

pub fn by_name(name: &str) -> Theme {
    THEMES.iter().copied().find(|t| t.name == name).unwrap_or(THEMES[0])
}
