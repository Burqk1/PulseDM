use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> io::Result<()> {
    while !data.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(file, data, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(file, data, offset)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        data = &data[n..];
        offset += n as u64;
    }
    Ok(())
}

pub fn create_part_file(path: &Path, size: Option<u64>) -> io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
    #[cfg(windows)]
    let _ = set_sparse(&file);
    if let Some(size) = size {
        file.set_len(size)?;
    }
    Ok(file)
}

pub fn open_part_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(windows)]
fn set_sparse(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;

    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(windows)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(available)
}

#[cfg(not(windows))]
pub fn free_space(_dir: &Path) -> Option<u64> {
    None
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

pub fn unique_name(dir: &Path, name: &str, part_ext: &str) -> String {
    let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}.{part_ext}")).exists();
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..).map(|i| format!("{stem} ({i}){ext}")).find(|n| !taken(n)).expect("infinite iterator")
}

pub fn part_path(dir: &Path, name: &str, part_ext: &str) -> PathBuf {
    dir.join(format!("{name}.{part_ext}"))
}

pub fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut s: String =
        base.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.chars().count() > 200 {
        let ext = s.rfind('.').map(|i| s[i..].to_string()).filter(|e| e.len() <= 16).unwrap_or_default();
        let stem: String = s.chars().take(200 - ext.chars().count()).collect();
        s = format!("{stem}{ext}");
    }
    let stem_upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let reserved = RESERVED.contains(&stem_upper.as_str())
        || ((stem_upper.starts_with("COM") || stem_upper.starts_with("LPT"))
            && stem_upper.len() == 4
            && stem_upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(0, '_');
    }
    if s.is_empty() { "download".to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_hostile_names() {
        assert_eq!(sanitize_file_name("a<b>:c?.zip"), "a_b__c_.zip");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("com1"), "_com1");
        assert_eq!(sanitize_file_name("name. . "), "name");
        assert_eq!(sanitize_file_name(""), "download");
    }

    #[test]
    fn unique_name_appends_counter() {
        let dir = std::env::temp_dir().join(format!("pulse-uniq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.zip"), b"").unwrap();
        std::fs::write(dir.join("a (1).zip.pdm"), b"").unwrap();
        assert_eq!(unique_name(&dir, "a.zip", "pdm"), "a (2).zip");
        assert_eq!(unique_name(&dir, "b.zip", "pdm"), "b.zip");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
