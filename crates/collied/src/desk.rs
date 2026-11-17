use std::path::Path;
use std::time::Duration;

use crate::runtime::{AgentInfo, Owner};

/// The shortest turn that alerts, so input this recent came after the turn began.
pub const ACTIVE_WITHIN: Duration = Duration::from_millis(crate::turns::MIN_TURN_MS);

/// The desktop session as macOS reports it.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    pub idle: Duration,
    pub locked: bool,
    pub on_console: bool,
    pub display_asleep: bool,
}

impl Session {
    pub fn active(&self) -> bool {
        self.on_console && !self.locked && !self.display_asleep && self.idle < ACTIVE_WITHIN
    }
}

/// herdr reports a finished turn `done` rather than `idle` when its tab is not active or the
/// client's terminal reported losing focus, unless herdr suppressed the completion (an agent
/// process exit ends `idle` whatever the focus). Herdr Core and Rex report no such thing.
pub fn herdr_saw(a: &AgentInfo) -> bool {
    Owner::of(&a.terminal_id) == Owner::Herdr && a.agent_status == "idle"
}

/// A done alert only repeats what the user sees when all three are known and true; any
/// unknown signal sends it.
pub fn in_front(
    pane_focused: Option<bool>,
    window_in_front: Option<bool>,
    user_active: Option<bool>,
) -> bool {
    pane_focused == Some(true) && window_in_front == Some(true) && user_active == Some(true)
}

/// The window is read only when the user is active.
#[cfg(target_os = "macos")]
pub fn desktop(client_socket: &Path) -> (Option<bool>, Option<bool>) {
    let active = mac::session().map(|s| s.active());
    if active != Some(true) {
        return (active, None);
    }
    let front = mac::front_pid().and_then(|pid| mac::hosts_client(pid, client_socket));
    (active, front)
}

#[cfg(not(target_os = "macos"))]
pub fn desktop(_client_socket: &Path) -> (Option<bool>, Option<bool>) {
    (None, None)
}

/// Public calls only, none behind a privacy prompt: CoreGraphics for the session and the
/// window list (owner and layer, never titles or pixels), libproc for process names and children, and for the sockets of the user's own processes.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::time::Duration;

    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};

    use super::Session;

    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const HID_SYSTEM_STATE: i32 = 1;
    const ANY_INPUT_EVENT: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static kCGWindowLayer: CFStringRef;
        static kCGWindowOwnerPID: CFStringRef;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> i32;
    }

    pub fn session() -> Option<Session> {
        let dict = unsafe { CGSessionCopyCurrentDictionary() };
        if dict.is_null() {
            return None;
        }
        let dict = unsafe { CFDictionary::<CFString, CFType>::wrap_under_create_rule(dict) };
        let flag = |key: &str| {
            dict.find(CFString::new(key))
                .and_then(|v| v.downcast::<CFBoolean>())
                .map(bool::from)
        };
        let idle =
            unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, ANY_INPUT_EVENT) };
        Some(Session {
            idle: Duration::try_from_secs_f64(idle).ok()?,
            // CGSession.h defines kCGSessionOnConsoleKey as this string.
            on_console: flag("kCGSSessionOnConsoleKey")?,
            // Undocumented: the lock screen sets it, and it is absent while unlocked.
            locked: flag("CGSSessionScreenIsLocked").unwrap_or(false),
            display_asleep: unsafe { CGDisplayIsAsleep(CGMainDisplayID()) } != 0,
        })
    }

    /// The owner of the frontmost normal window: the list runs front to back, and layer 0
    /// leaves out the menu bar, the Dock and floating panels.
    pub fn front_pid() -> Option<i32> {
        let list =
            unsafe { CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, 0) };
        if list.is_null() {
            return None;
        }
        let list =
            unsafe { CFArray::<CFDictionary<CFString, CFType>>::wrap_under_create_rule(list) };
        let layer = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
        let owner = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
        let number = |w: &CFDictionary<CFString, CFType>, key: &CFString| {
            w.find(key)?.downcast::<CFNumber>()?.to_i32()
        };
        let front = list.iter().find(|w| number(w, &layer) == Some(0))?;
        number(&front, &owner)
    }

    /// Past this many processes, or at pid 0 or 1 (pid 0 lists itself as its own child), a
    /// walk answers unknown, so it always ends.
    const MAX_PIDS: usize = 4096;

    pub fn hosts_client(root: i32, socket: &Path) -> Option<bool> {
        walk(root, socket, MAX_PIDS)
    }

    // Sockets are read only of processes named herdr, as its clients are: every other process
    // under the application is left alone.
    pub(super) fn walk(root: i32, socket: &Path, limit: usize) -> Option<bool> {
        let want = socket.as_os_str().as_bytes();
        let mut stack = vec![root];
        let mut visited = 0;
        while let Some(pid) = stack.pop() {
            visited += 1;
            if pid <= 1 || visited > limit {
                return None;
            }
            if is_herdr(pid) && connected_to(pid, want) {
                return Some(true);
            }
            stack.extend(children(pid));
        }
        Some(false)
    }

    fn is_herdr(pid: i32) -> bool {
        let mut name = [0u8; 64];
        let n = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
        n > 0 && name.get(..n as usize) == Some(b"herdr".as_slice())
    }

    fn children(pid: i32) -> Vec<i32> {
        let mut pids = vec![0i32; 1024];
        let size = (pids.len() * size_of::<i32>()) as i32;
        let n = unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr().cast(), size) };
        pids.truncate(n.max(0) as usize);
        pids
    }

    const PROC_PIDFDSOCKETINFO: i32 = 3;
    const SOCKINFO_UN: i32 = 3;
    // From <sys/proc_info.h> on arm64, checked with sizeof and offsetof: struct socket_fdinfo,
    // its psi.soi_kind, and its psi.soi_proto.pri_un.unsi_caddr, the peer's sockaddr_un.
    const SOCKET_FDINFO: usize = 792;
    const SOI_KIND: usize = 256;
    const UNSI_CADDR: usize = 535;
    const SUN_PATH: usize = 2;

    fn connected_to(pid: i32, want: &[u8]) -> bool {
        let fdinfo = size_of::<libc::proc_fdinfo>();
        let size =
            unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
        if size <= 0 {
            return false;
        }
        let mut fds = vec![
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0
            };
            size as usize / fdinfo
        ];
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast::<c_void>(),
                (fds.len() * fdinfo) as i32,
            )
        };
        fds.truncate(got.max(0) as usize / fdinfo);
        fds.iter()
            .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32)
            .any(|fd| peer(pid, fd.proc_fd).is_some_and(|p| p == want))
    }

    fn peer(pid: i32, fd: i32) -> Option<Vec<u8>> {
        let mut info = [0u64; SOCKET_FDINFO / 8];
        let got = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd,
                PROC_PIDFDSOCKETINFO,
                info.as_mut_ptr().cast(),
                SOCKET_FDINFO as i32,
            )
        };
        if got as usize != SOCKET_FDINFO {
            return None;
        }
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(info.as_ptr().cast(), SOCKET_FDINFO) };
        let kind = i32::from_ne_bytes(bytes[SOI_KIND..SOI_KIND + 4].try_into().ok()?);
        if kind != SOCKINFO_UN {
            return None;
        }
        let path = &bytes[UNSI_CADDR + SUN_PATH..UNSI_CADDR + SUN_PATH + 104];
        let len = path.iter().position(|&b| b == 0).unwrap_or(path.len());
        Some(path[..len].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNAL: [Option<bool>; 3] = [Some(true), Some(false), None];

    #[test]
    fn only_three_known_true_signals_skip_the_alert() {
        for focused in SIGNAL {
            for front in SIGNAL {
                for active in SIGNAL {
                    let all = focused == Some(true) && front == Some(true) && active == Some(true);
                    assert_eq!(
                        in_front(focused, front, active),
                        all,
                        "{focused:?} {front:?} {active:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_locked_asleep_away_or_absent_user_is_not_active() {
        let at_desk = Session {
            idle: Duration::from_secs(5),
            locked: false,
            on_console: true,
            display_asleep: false,
        };
        assert!(at_desk.active());
        let away = [
            Session {
                idle: ACTIVE_WITHIN,
                ..at_desk
            },
            Session {
                locked: true,
                ..at_desk
            },
            Session {
                on_console: false,
                ..at_desk
            },
            Session {
                display_asleep: true,
                ..at_desk
            },
        ];
        for s in away {
            assert!(!s.active(), "{s:?}");
        }
    }

    #[test]
    fn only_a_herdr_turn_herdr_saw_can_be_skipped() {
        let agent = |id: &str, status: &str| -> AgentInfo {
            serde_json::from_value(serde_json::json!({
                "terminal_id": id, "workspace_id": "w1", "pane_id": "w1:p1", "agent_status": status,
            }))
            .unwrap()
        };
        assert!(herdr_saw(&agent("term_1", "idle")));
        for (id, status) in [
            ("term_1", "done"),
            ("hc:core:t1", "idle"),
            ("rex:block:1", "idle"),
        ] {
            assert!(!herdr_saw(&agent(id, status)), "{id} {status}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finds_a_herdr_client_under_the_application_and_ends_unknown_at_its_bounds() {
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herdr-client.sock");
        let other = dir.path().join("herdr.sock");
        let _listener = UnixListener::bind(&path).unwrap();
        let _other = UnixListener::bind(&other).unwrap();
        let herdr = dir.path().join("herdr");
        std::fs::copy("/bin/sleep", &herdr).unwrap();
        let me = std::process::id() as i32;

        let stream = UnixStream::connect(&path).unwrap();
        assert_eq!(
            mac::hosts_client(me, &path),
            Some(false),
            "a process not named herdr is not a client"
        );
        let mut child = Command::new(&herdr)
            .arg("30")
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(stream)))
            .spawn()
            .unwrap();
        let found = mac::hosts_client(me, &path);
        let another = mac::hosts_client(me, &other);
        let capped = mac::walk(me, &path, 1);
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(found, Some(true), "a herdr child holds it");
        assert_eq!(another, Some(false));
        assert_eq!(capped, None, "past the limit");
        for root in [0, 1] {
            assert_eq!(mac::hosts_client(root, &path), None, "pid {root}");
        }
    }
}
