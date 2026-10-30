//! Startup check for the groups amdtop needs to open device nodes.
//!
//! The only device node amdtop opens itself is `/dev/accel/accelN`, which is
//! normally `root:render`. A user outside that group still sees CPU, GPU, memory
//! and process data, but the NPU panels come up empty, so they are warned before
//! the TUI takes over the screen.

use std::ffi::CStr;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// A device node the current user cannot open because of a missing group.
#[derive(Debug, Clone)]
pub struct Missing {
    pub user: String,
    pub group: String,
    pub gid: u32,
    pub node: PathBuf,
}

impl Missing {
    /// Human-readable warning, one line per entry.
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!(
                "{} is owned by group '{}' (gid {}), but {} is not a member.",
                self.node.display(),
                self.group,
                self.gid,
                self.user
            ),
            format!(
                "add the group with:  sudo usermod -aG {} {}",
                self.group, self.user
            ),
            "then log out and back in (group membership is fixed at login).".to_string(),
        ]
    }
}

/// Returns the missing access, if the NPU exists but this user cannot open it.
pub fn check() -> Option<Missing> {
    let node = accel_node()?;
    evaluate(&node, unsafe { libc::geteuid() }, &current_gids())
}

fn evaluate(node: &std::path::Path, euid: u32, gids: &[u32]) -> Option<Missing> {
    // Root can open the node regardless of its group.
    if euid == 0 {
        return None;
    }
    let gid = std::fs::metadata(node).ok()?.gid();
    if gids.contains(&gid) {
        return None;
    }
    Some(Missing {
        user: user_name(),
        group: group_name(gid),
        gid,
        node: node.to_path_buf(),
    })
}

/// First `/dev/accel/accel*` node, if the kernel exposes an accel device.
fn accel_node() -> Option<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/accel")
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("accel"))
        })
        .collect();
    nodes.sort();
    nodes.into_iter().next()
}

/// Supplementary group ids of this process, from `/proc/self/status`.
pub fn current_gids() -> Vec<u32> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Groups:"))
                .map(parse_groups)
        })
        .unwrap_or_default()
}

fn parse_groups(line: &str) -> Vec<u32> {
    line.trim_start_matches("Groups:")
        .split_whitespace()
        .filter_map(|value| value.parse::<u32>().ok())
        .collect()
}

fn group_name(gid: u32) -> String {
    let mut buffer = vec![0u8; 4096];
    let mut entry: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe {
        libc::getgrgid_r(
            gid,
            &mut entry,
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() {
        let name = unsafe { CStr::from_ptr(entry.gr_name) };
        name.to_string_lossy().into_owned()
    } else {
        gid.to_string()
    }
}

fn user_name() -> String {
    let mut buffer = vec![0u8; 4096];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut entry,
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() {
        let name = unsafe { CStr::from_ptr(entry.pw_name) };
        name.to_string_lossy().into_owned()
    } else {
        "this user".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_groups_line() {
        assert_eq!(parse_groups("Groups:\t1000 991 4 "), vec![1000, 991, 4]);
        assert_eq!(parse_groups("Groups:"), Vec::<u32>::new());
        assert_eq!(parse_groups("Groups: 27 108"), vec![27, 108]);
    }

    #[test]
    fn warning_names_the_fix() {
        let missing = Missing {
            user: "naal".into(),
            group: "render".into(),
            gid: 991,
            node: PathBuf::from("/dev/accel/accel0"),
        };
        let text = missing.lines().join("\n");
        assert!(text.contains("/dev/accel/accel0"));
        assert!(text.contains("'render'"));
        assert!(text.contains("sudo usermod -aG render naal"));
    }

    fn temp_node() -> (PathBuf, u32) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("amdtop-access-{}-{id}", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let gid = std::fs::metadata(&path).unwrap().gid();
        (path, gid)
    }

    #[test]
    fn warns_when_the_group_is_missing() {
        let (path, gid) = temp_node();
        let missing = evaluate(&path, 1000, &[4242, 4243]).expect("should warn");
        assert_eq!(missing.gid, gid);
        assert_eq!(missing.node, path);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stays_quiet_when_the_group_is_present() {
        let (path, gid) = temp_node();
        assert!(evaluate(&path, 1000, &[4242, gid]).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn root_needs_no_group() {
        let (path, _) = temp_node();
        assert!(evaluate(&path, 0, &[]).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn this_process_has_its_own_gid() {
        let gids = current_gids();
        assert!(!gids.is_empty(), "the groups line should not be empty");
        assert!(gids.contains(&unsafe { libc::getgid() }));
    }
}
