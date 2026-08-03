use std::ffi::OsStr;
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// One menu step. `Ok(None)` means the user cancelled, which is normal and quiet.
pub(crate) trait Menu {
    fn choose(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        height: u8,
    ) -> Result<Option<String>>;
    fn filter(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        placeholder: &str,
    ) -> Result<Option<String>>;
    fn input(
        &mut self,
        title: &str,
        subtitle: &str,
        placeholder: &str,
        width: u16,
        indent: InputIndent,
    ) -> Result<Option<String>>;
}

/// Where a `gum input` field starts.
///
/// The two input fields are indented differently in `scripts/new-agent-popup.zsh`: the
/// branch field is preceded by `printf '%*s' "$choice_margin"` (line 78) while the usage
/// field is not (line 130). The difference is visible on screen, so it is carried across
/// rather than tidied away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputIndent {
    Centered,
    None,
}

/// Strip the leading pad but keep the Nerd Font glyph.
///
/// Menu options carry leading spaces so `gum` renders them centered. The pad is removed
/// before the selection is used, while the glyph stays: the stripped label becomes the
/// pane and tab label, and dropping the glyph would make every tab look alike.
pub(crate) fn strip_pad(value: &str) -> String {
    value.trim_start().to_owned()
}

/// The width of `value` in terminal columns.
///
/// Centering pads have to agree with what `gum` draws, so this mirrors how `gum` measures
/// text. Verified with `gum style --border rounded`: `中文分支` renders 8 columns and
/// `こんにちは` 10, while the Nerd Font glyph in `\u{f15ce}  claude code` renders 1, so
/// that label measures 14. Only the East Asian wide blocks and emoji count double; the
/// private-use planes the Nerd Font glyphs live in do not.
fn display_width(value: &str) -> u16 {
    value.chars().fold(0, |total, character| {
        let width = match u32::from(character) {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        };
        total.saturating_add(width)
    })
}

/// Rows `gum filter` occupies, as both the flag value and the number vertical centering
/// has to reserve.
const FILTER_HEIGHT_ARG: &str = "12";
const FILTER_HEIGHT: u16 = 12;

pub(crate) struct GumMenu {
    cols: u16,
    lines: u16,
}

impl GumMenu {
    pub(crate) fn new(cols: u16, lines: u16) -> Self {
        Self { cols, lines }
    }

    /// The banner box width: 44, narrowed on a small viewport.
    fn content_width(&self) -> u16 {
        44.min(self.cols.saturating_sub(4))
    }

    /// The left margin that centers the banner box.
    ///
    /// The box draws two columns wider than `--width`: `gum style` counts the padding
    /// inside the width and adds one border column on each side. Measured at 80 columns,
    /// `--width 44` renders 46 and leaves 17 either side.
    fn content_margin(&self) -> u16 {
        self.cols
            .saturating_sub(self.content_width())
            .saturating_sub(2)
            / 2
    }

    /// The left margin that centers a block `width` columns wide.
    fn block_margin(&self, width: u16) -> u16 {
        self.cols.saturating_sub(width) / 2
    }

    /// The blank rows that center a block `height` rows tall.
    fn vertical_padding(&self, height: u16) -> u16 {
        self.lines.saturating_sub(height) / 2
    }

    /// Draw the centered banner: title, blank line, dim subtitle.
    ///
    /// The banner is printed line by line at a computed margin instead of being handed to
    /// another `gum` call. Wrapping an already-styled multiline banner in a second
    /// `gum style` offsets its border lines, because the outer call measures the ANSI
    /// escapes as visible width and pads each line differently — the box comes out
    /// ragged. Printing the lines here keeps the border square.
    fn render_banner(&self, title: &str, subtitle: &str, body_lines: u16) -> Result<()> {
        if io::stdout().is_terminal() {
            print!("\x1b[2J\x1b[H");
        }
        let width = self.content_width();
        let subtitle = gum_output(["style", "--foreground", "240", subtitle])?.unwrap_or_default();
        let banner = gum_output([
            "style",
            "--border",
            "rounded",
            "--padding",
            "1 3",
            "--width",
            &width.to_string(),
            "--bold",
            title,
            "",
            subtitle.trim_end(),
        ])?
        .unwrap_or_default();
        // The banner is measured rather than assumed: a narrow viewport wraps the subtitle
        // and adds rows. `+ 1` is the blank line this prints between banner and body.
        let banner_lines = u16::try_from(banner.lines().count()).unwrap_or(u16::MAX);
        let block = banner_lines.saturating_add(1).saturating_add(body_lines);
        print!("{}", "\n".repeat(self.vertical_padding(block).into()));
        let margin = usize::from(self.content_margin());
        for line in banner.lines() {
            println!("{:margin$}{line}", "");
        }
        println!();
        io::stdout().flush().context("failed to draw menu banner")
    }

    /// Indent every option by one shared margin so the block is centered and its glyphs
    /// stay in a single column. `gum choose` runs with an empty `--cursor`, so it adds no
    /// prefix of its own and an option starts exactly at this margin.
    fn padded(&self, options: &[String]) -> Vec<String> {
        let widest = options.iter().map(|option| display_width(option)).max();
        let pad = " ".repeat(usize::from(self.block_margin(widest.unwrap_or(0))));
        options
            .iter()
            .map(|option| format!("{pad}{option}"))
            .collect()
    }

    /// The full `gum choose` argv for these options.
    ///
    /// The `--` is load-bearing: an option that starts with a dash is parsed as a flag
    /// otherwise. The centering pad usually hides that, but the widest option gets no pad
    /// at all, so a label like `--help` would reach `gum` as a flag — printing help,
    /// exiting zero, and returning a selection that matches no entry.
    fn choose_args(&self, options: &[String], height: u8) -> Vec<String> {
        let mut args = vec![
            "choose".to_owned(),
            "--height".to_owned(),
            height.to_string(),
            "--no-show-help".to_owned(),
            "--cursor".to_owned(),
            String::new(),
            "--header".to_owned(),
            String::new(),
            "--".to_owned(),
        ];
        args.extend(self.padded(options));
        args
    }
}

/// The popup's viewport in columns and rows, for sizing a [`GumMenu`].
pub(crate) fn popup_viewport() -> (u16, u16) {
    super::terminal_size().unwrap_or_else(|| {
        (
            viewport_dimension("COLUMNS", "cols"),
            viewport_dimension("LINES", "lines"),
        )
    })
}

fn viewport_dimension(variable: &str, tput_capability: &str) -> u16 {
    super::nonempty_env(variable)
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0)
        .or_else(|| {
            Command::new("tput")
                .arg(tput_capability)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|value| value.trim().parse::<u16>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(80)
}

impl Menu for GumMenu {
    fn choose(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        height: u8,
    ) -> Result<Option<String>> {
        // `gum choose` draws one row per option and never pads out to `--height`.
        let rows = u16::try_from(options.len()).unwrap_or(u16::MAX);
        self.render_banner(title, subtitle, rows.min(u16::from(height)))?;
        gum_output(self.choose_args(options, height))
    }

    fn filter(
        &mut self,
        title: &str,
        subtitle: &str,
        options: &[String],
        placeholder: &str,
    ) -> Result<Option<String>> {
        // Unlike `choose`, `gum filter`'s `--height` is the whole frame: the query line,
        // the list, and the help line. It always occupies that many rows.
        self.render_banner(title, subtitle, FILTER_HEIGHT)?;
        // --no-strict returns the typed text when it matches no branch, so the same field
        // both picks an existing branch and names a new one.
        gum_with_input(
            &[
                "filter",
                "--no-strict",
                "--height",
                FILTER_HEIGHT_ARG,
                "--placeholder",
                placeholder,
            ],
            &self.padded(options).join("\n"),
        )
    }

    fn input(
        &mut self,
        title: &str,
        subtitle: &str,
        placeholder: &str,
        width: u16,
        indent: InputIndent,
    ) -> Result<Option<String>> {
        self.render_banner(title, subtitle, 1)?;
        if indent == InputIndent::Centered {
            print!("{}", " ".repeat(usize::from(self.block_margin(width))));
            io::stdout().flush().context("failed to indent gum input")?;
        }
        gum_output([
            "input",
            "--placeholder",
            placeholder,
            "--width",
            &width.to_string(),
        ])
    }
}

/// Run `gum` and capture its selection.
///
/// `gum` writes the selection to stdout but draws its UI on *stderr* whenever stdout
/// is not a terminal — which is exactly our case, since we capture the selection. So
/// stderr must be inherited or the menu renders nowhere and the user chooses blind.
/// A non-zero exit means the user cancelled: `Ok(None)`, never an error.
fn gum_output<I, S>(args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("gum")
        .args(args)
        .stderr(Stdio::inherit())
        .output()
        .context("failed to run gum")?;
    Ok(output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }))
}

/// Same contract as [`gum_output`], for the filter menu whose options arrive on stdin.
fn gum_with_input(args: &[&str], input: &str) -> Result<Option<String>> {
    let mut child = Command::new("gum")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to run gum")?;
    child
        .stdin
        .take()
        .context("failed to open gum input")?
        .write_all(input.as_bytes())
        .context("failed to write gum options")?;
    let output = child
        .wait_with_output()
        .context("failed to read gum selection")?;
    Ok(output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centering_geometry_matches_the_popup() {
        let menu = GumMenu::new(80, 40);
        assert_eq!(menu.content_width(), 44);
        // `--width 44` draws 46 columns, so 17 either side of an 80-column viewport.
        assert_eq!(menu.content_margin(), 17);
        assert_eq!(80 - menu.content_margin() - (menu.content_width() + 2), 17);
        // A 24-column block leaves 28 either side; the old fixed choice margin.
        assert_eq!(menu.block_margin(24), 28);
        assert_eq!(
            menu.padded(&["\u{ead8}  debug".to_owned()]),
            [format!("{}\u{ead8}  debug", " ".repeat(36))]
        );

        // A viewport too narrow for the banner narrows the box and floors both margins.
        let menu = GumMenu::new(20, 8);
        assert_eq!(menu.content_width(), 16);
        assert_eq!(menu.content_margin(), 1);
        assert_eq!(menu.block_margin(24), 0);
    }

    #[test]
    fn the_block_is_centered_on_the_rows_it_occupies() {
        // The banner is 7 rows, plus the blank line, plus the menu body.
        let menu = GumMenu::new(80, 40);
        assert_eq!(menu.vertical_padding(7 + 1 + 3), 14);
        assert_eq!(40 - menu.vertical_padding(11) - 11, 15);
        // The worktree filter reserves its whole frame, so it sits higher.
        assert_eq!(menu.vertical_padding(7 + 1 + FILTER_HEIGHT), 10);

        // A viewport shorter than the block floors the padding instead of scrolling.
        assert_eq!(GumMenu::new(80, 10).vertical_padding(11), 0);
    }

    #[test]
    fn the_option_block_is_centered_on_its_widest_option() {
        let menu = GumMenu::new(80, 40);
        let options = [
            "\u{f15ce}  claude code".to_owned(),
            "\u{ee0d}  codex".to_owned(),
            "\u{f169f}  opencode".to_owned(),
        ];
        // The widest option is 14 columns, so the block starts at 33 and ends at 47.
        let padded = menu.padded(&options);
        let pad = " ".repeat(33);
        assert!(
            padded.iter().all(|option| option.starts_with(&pad)),
            "every option shares one left edge so the glyphs line up: {padded:?}"
        );
        assert_eq!(display_width(&padded[0]), 47);
        assert_eq!(80 - display_width(&padded[0]), 33);

        // A narrower option set moves further right; the old fixed 24 could not.
        let model = menu.padded(&["Fable 5".to_owned()]);
        assert_eq!(display_width(&model[0]) - 7, 36);
    }

    #[test]
    fn every_option_follows_the_flag_terminator() {
        // A viewport no wider than the widest option leaves the centering pad empty, so a
        // dash-prefixed label reaches gum bare. Without `--` gum reads it as a flag,
        // prints its help, exits zero, and returns a selection that matches no entry.
        let menu = GumMenu::new(6, 40);
        let args = menu.choose_args(&["--help".to_owned(), "ok".to_owned()], 8);
        let terminator = args
            .iter()
            .position(|arg| arg == "--")
            .expect("choose_args ends its flags with --");
        assert_eq!(&args[terminator + 1..], ["--help", "ok"]);
        assert!(
            args[..terminator].iter().all(|arg| arg != "--help"),
            "no option may sit among the flags: {args:?}"
        );
    }

    #[test]
    fn a_wide_character_counts_as_two_columns() {
        // Measured against `gum style --border rounded`, which draws the same widths.
        assert_eq!(display_width("中文分支"), 8);
        assert_eq!(display_width("こんにちは"), 10);
        assert_eq!(display_width("feat/中文"), 9);
        // A Nerd Font glyph is one column, so these labels measure as plain text.
        assert_eq!(display_width("\u{f15ce}  claude code"), 14);
        assert_eq!(display_width("\u{f19b9}  let me write…"), 16);
    }
}
