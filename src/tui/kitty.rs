use std::{
    env,
    io::{self, Write},
};

use base64::{Engine, engine::general_purpose::STANDARD};

use super::view::Layout;

const IMAGE_IDS: [u32; 2] = [42, 43];
const CHUNK_SIZE: usize = 4096;

pub fn supported() -> bool {
    env::var_os("KITTY_WINDOW_ID").is_some_and(|value| !value.is_empty())
        || env::var("TERM").as_deref() == Ok("xterm-kitty")
        || matches!(
            env::var("TERM_PROGRAM").as_deref(),
            Ok("ghostty" | "WezTerm")
        )
}

#[derive(Default)]
pub struct Preview {
    visible: Option<u32>,
}

impl Preview {
    pub fn clear(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if let Some(id) = self.visible.take() {
            delete(writer, id)?;
            writer.flush()?;
        }
        Ok(())
    }

    pub fn draw(&mut self, writer: &mut impl Write, png: &[u8], layout: Layout) -> io::Result<()> {
        let Some((row, col)) = layout.placement() else {
            return Ok(());
        };
        if png.is_empty() {
            return Ok(());
        }

        // Kitty (and some compatible terminals) can drop an image as soon
        // as its ID is reused. Keep it displayed during the chunked upload
        // of the other ID, and retire it only after the new placement exists.
        let next = IMAGE_IDS[usize::from(self.visible == Some(IMAGE_IDS[0]))];
        let payload = STANDARD.encode(png);
        let chunks = payload.len().div_ceil(CHUNK_SIZE);
        // DECSC/DECRC keep the image positioning out of Ratatui's text diff.
        write!(writer, "\x1b7\x1b[{row};{col}H")?;
        for (i, chunk) in payload.as_bytes().chunks(CHUNK_SIZE).enumerate() {
            let more = usize::from(i + 1 < chunks);
            if i == 0 {
                write!(
                    writer,
                    "\x1b_Gf=100,a=T,i={next},c={},r={},z=-1,C=1,q=2,m={more};",
                    layout.cols, layout.rows
                )?;
            } else {
                // WezTerm uses the final chunk's quiet setting on completion.
                write!(writer, "\x1b_Gm={more},q=2;")?;
            }
            writer.write_all(chunk)?;
            writer.write_all(b"\x1b\\")?;
        }
        if let Some(previous) = self.visible.replace(next) {
            delete(writer, previous)?;
        }
        writer.write_all(b"\x1b8")?;
        writer.flush()
    }
}

fn delete(writer: &mut impl Write, id: u32) -> io::Result<()> {
    // Delete only vroom's image and its stored data, not unrelated placements.
    write!(writer, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
}

pub fn cleanup(writer: &mut impl Write) -> io::Result<()> {
    // On exit, try both IDs in case an I/O error interrupted a swap.
    for id in IMAGE_IDS {
        delete(writer, id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Size;

    use super::*;
    use crate::video::Info;

    fn layout() -> Layout {
        Layout::new(
            Size::new(80, 41),
            Info {
                duration: 12.0,
                fps: 25.0,
                width: 1920,
                height: 1080,
            },
        )
    }

    #[test]
    fn encodes_kitty_png_underneath_text_with_saved_cursor_and_4096_byte_chunks() -> io::Result<()>
    {
        for png_len in [3072, 3073] {
            let mut output = Vec::new();
            Preview::default().draw(&mut output, &vec![0; png_len], layout())?;
            let output = String::from_utf8(output).unwrap();
            let (prefix, payload) = output.split_once(";AAAA").unwrap();
            assert!(prefix.starts_with("\x1b7\x1b[8;4H"));
            assert!(!output.contains("a=d"));
            assert!(prefix.contains("f=100,a=T,i=42,c=74,r=21,z=-1,C=1,q=2"));
            assert!(output.ends_with("\x1b\\\x1b8"));
            let (first, rest) = payload.split_once("\x1b\\").unwrap();
            assert_eq!(first.len() + 4, CHUNK_SIZE); // split_once removed "AAAA"
            if png_len == 3072 {
                assert!(prefix.ends_with("m=0"));
                assert_eq!(rest, "\x1b8");
            } else {
                assert!(prefix.ends_with("m=1"));
                assert_eq!(rest, "\x1b_Gm=0,q=2;AA==\x1b\\\x1b8");
            }
        }
        Ok(())
    }

    #[test]
    fn swaps_two_images_without_deleting_the_visible_frame_during_upload() -> io::Result<()> {
        let layout = layout();
        let mut preview = Preview::default();
        let mut bytes = Vec::new();
        for (next, old) in [(42, None), (43, Some(42)), (42, Some(43))] {
            bytes.clear();
            preview.draw(&mut bytes, &vec![0; 3073], layout)?;
            let wire = std::str::from_utf8(&bytes).unwrap();
            assert!(wire.starts_with(&format!("\x1b7\x1b[8;4H\x1b_Gf=100,a=T,i={next},")));
            let (_, tail) = wire.split_once("\x1b_Gm=0,q=2;AA==\x1b\\").unwrap();
            let expected =
                old.map_or_else(String::new, |id| format!("\x1b_Gq=2,a=d,d=I,i={id}\x1b\\"));
            assert_eq!(tail, format!("{expected}\x1b8"));
            assert_eq!(wire.matches("a=d").count(), usize::from(old.is_some()));
        }
        bytes.clear();
        preview.clear(&mut bytes)?;
        preview.clear(&mut bytes)?;
        assert_eq!(bytes, b"\x1b_Gq=2,a=d,d=I,i=42\x1b\\");
        bytes.clear();
        cleanup(&mut bytes)?;
        assert_eq!(
            bytes,
            b"\x1b_Gq=2,a=d,d=I,i=42\x1b\\\x1b_Gq=2,a=d,d=I,i=43\x1b\\"
        );
        Ok(())
    }
}
