use std::io::{self, Write};

const BELL_CHUNK: [u8; 64] = [b'\x07'; 64];

pub(crate) fn write_terminal_bells<W: Write>(writer: &mut W, count: u16) -> io::Result<()> {
    let full_chunks = usize::from(count) / BELL_CHUNK.len();
    let remainder = usize::from(count) % BELL_CHUNK.len();
    for _ in 0..full_chunks {
        writer.write_all(&BELL_CHUNK)?;
    }
    writer.write_all(&BELL_CHUNK[..remainder])?;
    writer.flush()
}

/// orcha painted the host terminal's default background (OSC 11), so the
/// window padding matches its page; reset it (OSC 111) on the way out.
static HOST_BACKGROUND_SET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn write_host_background<W: Write>(writer: &mut W, rgb: (u8, u8, u8)) -> io::Result<()> {
    let (r, g, b) = rgb;
    write!(writer, "\x1b]11;rgb:{r:02x}/{g:02x}/{b:02x}\x07")?;
    HOST_BACKGROUND_SET.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.flush()
}

pub(crate) fn reset_host_background<W: Write>(writer: &mut W) -> io::Result<()> {
    if HOST_BACKGROUND_SET.swap(false, std::sync::atomic::Ordering::Relaxed) {
        write!(writer, "\x1b]111\x07")?;
        writer.flush()?;
    }
    Ok(())
}

pub(crate) fn write_window_title<W: Write>(writer: &mut W, title: Option<&str>) -> io::Result<()> {
    let title = title.unwrap_or("herdr");
    let safe_title = title
        .chars()
        .filter(|ch| !matches!(*ch, '\u{1b}' | '\u{7}' | '\u{9c}'))
        .collect::<String>();
    write!(writer, "\x1b]0;{safe_title}\x07")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_exact_terminal_bell_count() {
        let mut output = Vec::new();

        write_terminal_bells(&mut output, 130).unwrap();

        assert_eq!(output, vec![b'\x07'; 130]);
    }

    #[test]
    fn window_title_strips_terminators_and_defaults_to_herdr() {
        let mut output = Vec::new();
        write_window_title(&mut output, Some("herdr\x1b api\u{7}\u{9c}")).unwrap();
        assert_eq!(output, b"\x1b]0;herdr api\x07");

        output.clear();
        write_window_title(&mut output, None).unwrap();
        assert_eq!(output, b"\x1b]0;herdr\x07");
    }
}
