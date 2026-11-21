//! Picks where captures are saved.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// ~/Pictures/snapit
pub fn default_dir() -> PathBuf {
    home().join("Pictures").join("snapit")
}

/// Turns a leading "~/" into the user's home directory.
pub fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(&rest[1..]),
        _ => PathBuf::from(p),
    }
}

/// Creates dir (and parents) if missing and checks it is a directory.
pub fn ensure_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if !std::fs::metadata(dir)?.is_dir() {
        return Err(io::Error::other(format!("{} is not a directory", dir.display())));
    }
    Ok(())
}

/// Local wall-clock time, broken down. std has no time zones, so this asks libc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
}

impl Stamp {
    pub fn now() -> Stamp {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        // SAFETY: localtime_r only writes into the tm we pass.
        unsafe {
            let t = secs as libc_time_t;
            let mut tm: Tm = std::mem::zeroed();
            localtime_r(&t, &mut tm);
            Stamp {
                year: tm.tm_year + 1900,
                month: (tm.tm_mon + 1) as u32,
                day: tm.tm_mday as u32,
                hour: tm.tm_hour as u32,
                min: tm.tm_min as u32,
                sec: tm.tm_sec as u32,
            }
        }
    }
}

#[allow(non_camel_case_types)]
type libc_time_t = i64;

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const libc_time_t, out: *mut Tm) -> *mut Tm;
}

/// dir/snapit-YYYY-MM-DD-HHMMSS.png, adding -1, -2, … when a file with that
/// name already exists.
pub fn next_path(dir: &Path, t: Stamp) -> io::Result<PathBuf> {
    let base = format!(
        "snapit-{:04}-{:02}-{:02}-{:02}{:02}{:02}",
        t.year, t.month, t.day, t.hour, t.min, t.sec
    );
    for n in 0..1000 {
        let name = if n == 0 { format!("{base}.png") } else { format!("{base}-{n}.png") };
        let p = dir.join(name);
        match std::fs::symlink_metadata(&p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(p),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    Err(io::Error::other(format!("too many captures named {base} in {}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("snapit-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn next_path_collisions() {
        let dir = tmp("next");
        ensure_dir(&dir).unwrap();
        let ts = Stamp { year: 2026, month: 11, day: 20, hour: 22, min: 15, sec: 30 };
        let mut p = next_path(&dir, ts).unwrap();
        assert_eq!(p, dir.join("snapit-2026-11-20-221530.png"));
        for want in ["snapit-2026-11-20-221530-1.png", "snapit-2026-11-20-221530-2.png"] {
            std::fs::write(&p, b"").unwrap();
            p = next_path(&dir, ts).unwrap();
            assert_eq!(p.file_name().unwrap(), want);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_dir_rejects_file() {
        let dir = tmp("ensure").join("a/b");
        ensure_dir(&dir).unwrap();
        let f = dir.join("file");
        std::fs::write(&f, b"").unwrap();
        assert!(ensure_dir(&f).is_err());
    }

    #[test]
    fn expand() {
        assert_eq!(expand_home("~/Pictures/x"), home().join("Pictures/x"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
        assert_eq!(expand_home("~"), home());
    }

    #[test]
    fn stamp_now_is_sane() {
        let s = Stamp::now();
        assert!(s.year >= 2024 && (1..=12).contains(&s.month) && s.hour < 24);
    }
}
