use std::io;

/// Kernel TCP sockets in LISTEN state held by `pid`, one line each. Empty means the
/// process listens only through the tailnet. Any read error is an error, never empty.
pub fn kernel_tcp_listeners(pid: u32) -> io::Result<Vec<String>> {
    alive(pid)?;
    imp::listeners(pid)
}

fn alive(pid: u32) -> io::Result<()> {
    let ok = i32::try_from(pid)
        .ok()
        .filter(|p| *p > 0)
        // SAFETY: signal 0 only checks that the process exists and may be signalled.
        .is_some_and(|p| unsafe { libc::kill(p, 0) } == 0);
    if ok {
        Ok(())
    } else {
        Err(io::Error::other(format!("pid {pid} is not running")))
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::io;

    // lsof prints nothing and exits 1 both when nothing matches and when the pid is gone,
    // so an empty result only counts once the pid is known to be alive and lsof stayed
    // silent.
    pub fn listeners(pid: u32) -> io::Result<Vec<String>> {
        let out = std::process::Command::new("/usr/sbin/lsof")
            .args(["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:LISTEN"])
            .output()?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.trim().is_empty() || !matches!(out.status.code(), Some(0 | 1)) {
            return Err(io::Error::other(format!(
                "lsof: {} ({})",
                stderr.trim(),
                out.status
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .skip(1)
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::BTreeSet;
    use std::io;

    const LISTEN: &str = "0A";

    pub fn listeners(pid: u32) -> io::Result<Vec<String>> {
        let inodes = socket_inodes(pid)?;
        let mut found = Vec::new();
        for table in ["tcp", "tcp6"] {
            let path = format!("/proc/{pid}/net/{table}");
            let text = std::fs::read_to_string(&path)
                .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
            found.extend(
                listening(&text, &inodes, table).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("{path}: {e}"))
                })?,
            );
        }
        Ok(found)
    }

    fn socket_inodes(pid: u32) -> io::Result<BTreeSet<u64>> {
        let dir = format!("/proc/{pid}/fd");
        let entries =
            std::fs::read_dir(&dir).map_err(|e| io::Error::new(e.kind(), format!("{dir}: {e}")))?;
        let mut inodes = BTreeSet::new();
        for entry in entries {
            let entry = entry?;
            let target = match std::fs::read_link(entry.path()) {
                Ok(t) => t,
                // Closed since the listing: no longer held, so not a listener.
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("{}: {e}", entry.path().display()),
                    ));
                }
            };
            if let Some(inode) = target
                .to_str()
                .and_then(|t| t.strip_prefix("socket:["))
                .and_then(|t| t.strip_suffix(']'))
            {
                inodes.insert(inode.parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("fd link {target:?}"))
                })?);
            }
        }
        Ok(inodes)
    }

    // The header names tx_queue rx_queue and tr tm->when separately, the rows join each
    // pair with a colon: inode is header column 11 and row field 9.
    pub(super) fn listening(
        text: &str,
        inodes: &BTreeSet<u64>,
        table: &str,
    ) -> Result<Vec<String>, String> {
        let mut lines = text.lines();
        let header = lines.next().ok_or("empty table")?;
        let columns: Vec<&str> = header.split_whitespace().collect();
        if columns.get(1) != Some(&"local_address")
            || columns.get(3) != Some(&"st")
            || columns.get(11) != Some(&"inode")
        {
            return Err(format!("unexpected header {header:?}"));
        }
        let mut found = Vec::new();
        for line in lines {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(state), Some(inode)) =
                (fields.get(1), fields.get(3), fields.get(9))
            else {
                return Err(format!("short row {line:?}"));
            };
            let inode: u64 = inode.parse().map_err(|_| format!("row {line:?}"))?;
            if *state == LISTEN && inodes.contains(&inode) {
                found.push(format!("{table} {local} inode {inode}"));
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: a listener bound by a parallel test would show up in the others.
    #[test]
    fn needs_a_live_pid_and_sees_a_kernel_listener() {
        assert_eq!(
            kernel_tcp_listeners(std::process::id()).unwrap(),
            Vec::<String>::new()
        );
        assert!(kernel_tcp_listeners(999_999).is_err());
        assert!(kernel_tcp_listeners(u32::MAX).is_err());
        assert!(kernel_tcp_listeners(0).is_err());

        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let found = kernel_tcp_listeners(std::process::id()).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        drop(l);
        let v6 = std::net::TcpListener::bind("[::1]:0");
        if let Ok(l) = v6 {
            assert_eq!(kernel_tcp_listeners(std::process::id()).unwrap().len(), 1);
            drop(l);
        }
        assert_eq!(
            kernel_tcp_listeners(std::process::id()).unwrap(),
            Vec::<String>::new()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_proc_net_tcp() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F95 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0000000000000000 100 0 0 10 0
   1: 0100007F:1F96 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1000        0 4243 1 0000000000000000 20 4 30 10 -1
   2: 00000000:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 99 1 0000000000000000 100 0 0 10 0
";
        let ours = [4242, 4243].into_iter().collect();
        assert_eq!(
            imp::listening(text, &ours, "tcp").unwrap(),
            vec!["tcp 0100007F:1F95 inode 4242".to_owned()]
        );
        assert!(imp::listening("garbage\n", &ours, "tcp").is_err());
        assert!(imp::listening("", &ours, "tcp").is_err());
    }
}
