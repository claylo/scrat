//! Terminal image rendering via inline graphics protocols.
//!
//! Supports the Kitty graphics protocol (kitty, ghostty, WezTerm) and
//! iTerm2 inline images protocol (iTerm.app, WezTerm). Falls back
//! gracefully when the terminal supports neither.

use std::io::Write;

use base64::{Engine, engine::general_purpose::STANDARD};

/// The :shipit: squirrel, embedded at compile time.
const SHIPIT_PNG: &[u8] = include_bytes!("../assets/shipit.png");

/// Display width in terminal columns for the squirrel image.
const IMG_COLS: u32 = 10;
/// Display height in terminal rows. Terminal cells are roughly 2:1
/// (height:width), so a square image at N columns needs ~N/2 rows.
const IMG_ROWS: u32 = 5;

/// Try to render the :shipit: squirrel inline.
///
/// Returns `true` if the image was rendered, `false` if the terminal
/// doesn't support inline graphics (caller should use a text fallback).
pub fn render_shipit() -> bool {
    try_kitty_render(SHIPIT_PNG)
        .or_else(|| try_iterm2_render(SHIPIT_PNG))
        .is_some()
}

/// Detect Kitty graphics protocol support via environment variables.
fn is_kitty_capable() -> bool {
    matches!(std::env::var("TERM").as_deref(), Ok("xterm-kitty"))
        || matches!(
            std::env::var("TERM_PROGRAM").as_deref(),
            Ok("kitty" | "ghostty" | "WezTerm")
        )
}

/// Detect iTerm2 inline images protocol support via environment variables.
fn is_iterm2_capable() -> bool {
    matches!(
        std::env::var("TERM_PROGRAM").as_deref(),
        Ok("iTerm.app" | "WezTerm")
    ) || matches!(std::env::var("LC_TERMINAL").as_deref(), Ok("iTerm2"))
}

/// Render a PNG using the Kitty graphics protocol.
///
/// Sends raw PNG bytes base64-encoded, chunked at 4096 bytes.
/// Format: `ESC _G <params>;<base64_chunk> ESC \`
fn try_kitty_render(png_bytes: &[u8]) -> Option<()> {
    if !is_kitty_capable() {
        return None;
    }

    let encoded = STANDARD.encode(png_bytes);
    let mut stdout = std::io::stdout().lock();

    // Kitty protocol chunks base64 data at 4096 bytes
    let chunk_size = 4096;
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(chunk_size).collect();

    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i == chunks.len() - 1;
        let more = if is_last { 0 } else { 1 };

        if i == 0 {
            // First chunk: a=T (transmit+display), f=100 (PNG)
            // c/r set cell dimensions; r = c/2 compensates for ~2:1 cell aspect ratio
            let _ = write!(
                stdout,
                "\x1b_Ga=T,f=100,c={IMG_COLS},r={IMG_ROWS},m={more};"
            );
        } else {
            let _ = write!(stdout, "\x1b_Gm={more};");
        }
        let _ = stdout.write_all(chunk);
        let _ = write!(stdout, "\x1b\\");
    }

    let _ = writeln!(stdout);
    let _ = stdout.flush();
    Some(())
}

/// Render a PNG using the iTerm2 inline images protocol.
///
/// Format: `ESC ]1337;File=inline=1;width=20;preserveAspectRatio=1:<base64_data> BEL`
fn try_iterm2_render(png_bytes: &[u8]) -> Option<()> {
    if !is_iterm2_capable() {
        return None;
    }

    let encoded = STANDARD.encode(png_bytes);
    let mut stdout = std::io::stdout().lock();

    // width/height in character cells; preserveAspectRatio stretches to fit
    let _ = write!(
        stdout,
        "\x1b]1337;File=inline=1;width={IMG_COLS};height={IMG_ROWS}:{encoded}\x07"
    );
    let _ = writeln!(stdout);
    let _ = stdout.flush();
    Some(())
}
