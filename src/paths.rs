use std::{
    ffi::OsString,
    path::{Path, PathBuf, is_separator},
};

pub fn save_path(source: &Path) -> PathBuf {
    with_suffix(source, ".vroom", false)
}

pub fn export_path(source: &Path) -> PathBuf {
    with_suffix(source, "_vroom", true)
}

pub fn log_path(output: &Path) -> PathBuf {
    with_suffix(output, ".log", false)
}

fn with_suffix(path: &Path, suffix: &str, keep_extension: bool) -> PathBuf {
    // Match the original filepath.Ext behavior, including dotfiles, and
    // preserve the original path spelling and non-UTF-8 bytes.
    let name = path.as_os_str();
    let bytes = name.as_encoded_bytes();
    let extension = bytes
        .iter()
        .enumerate()
        .rev()
        .take_while(|&(_, &byte)| !is_separator(char::from(byte)))
        .find_map(|(i, &byte)| (byte == b'.').then_some(i))
        .unwrap_or(bytes.len());
    let mut result = Vec::with_capacity(bytes.len() + suffix.len());
    result.extend_from_slice(&bytes[..extension]);
    result.extend_from_slice(suffix.as_bytes());
    if keep_extension {
        result.extend_from_slice(&bytes[extension..]);
    }
    // SAFETY: The slices come from as_encoded_bytes() and are separated only
    // at the start of an ASCII dot, or at the end. The inserted bytes are UTF-8.
    unsafe { OsString::from_encoded_bytes_unchecked(result) }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_derived_from_the_last_extension_only() {
        for (source, save, output, log) in [
            (
                "./some.dir//my clip.v2.MP4",
                "./some.dir//my clip.v2.vroom",
                "./some.dir//my clip.v2_vroom.MP4",
                "./some.dir//my clip.v2_vroom.log",
            ),
            (
                "some.dir/clip",
                "some.dir/clip.vroom",
                "some.dir/clip_vroom",
                "some.dir/clip_vroom.log",
            ),
            (".hidden", ".vroom", "_vroom.hidden", "_vroom.log"),
            ("clip.", "clip.vroom", "clip_vroom.", "clip_vroom.log"),
        ] {
            let source = Path::new(source);
            assert_eq!(save_path(source).as_os_str(), save);
            assert_eq!(export_path(source).as_os_str(), output);
            assert_eq!(log_path(Path::new(output)).as_os_str(), log);
        }
    }

    #[cfg(unix)]
    #[test]
    fn filenames_need_not_be_utf8() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let source = Path::new(OsStr::from_bytes(b"clip-\xff.mp4"));
        assert_eq!(
            export_path(source).as_os_str().as_bytes(),
            b"clip-\xff_vroom.mp4"
        );
    }
}
