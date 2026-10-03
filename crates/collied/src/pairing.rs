use std::time::{Duration, Instant};

use base64::Engine;
use protocol::PairingCode;
use subtle::ConstantTimeEq;

pub const WINDOW_TTL: Duration = Duration::from_secs(120);

pub fn new_code() -> std::io::Result<PairingCode> {
    let mut bytes = zeroize::Zeroizing::new([0u8; protocol::limits::PAIRING_CODE_BYTES]);
    getrandom::fill(bytes.as_mut()).map_err(|e| std::io::Error::other(e.to_string()))?;
    let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes.as_ref());
    PairingCode::new(text).map_err(|e| std::io::Error::other(e.to_string()))
}

struct Window<T> {
    id: u64,
    code: PairingCode,
    expires_at: Instant,
    owner: T,
}

pub struct Pairing<T> {
    window: Option<Window<T>>,
    next_id: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Busy;

#[derive(Debug, PartialEq, Eq)]
pub enum Attempt<T> {
    NoWindow,
    WrongCode(T),
    Accepted(T),
}

impl<T> Default for Pairing<T> {
    fn default() -> Self {
        Self {
            window: None,
            next_id: 1,
        }
    }
}

impl<T> Pairing<T> {
    pub fn open(&mut self, code: PairingCode, now: Instant, owner: T) -> Result<u64, Busy> {
        if self.current(now).is_some() {
            return Err(Busy);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.window = Some(Window {
            id,
            code,
            expires_at: now + WINDOW_TTL,
            owner,
        });
        Ok(id)
    }

    pub fn current(&self, now: Instant) -> Option<u64> {
        self.window
            .as_ref()
            .filter(|w| now < w.expires_at)
            .map(|w| w.id)
    }

    pub fn close(&mut self, id: u64) {
        if self.window.as_ref().is_some_and(|w| w.id == id) {
            self.window = None;
        }
    }

    /// Every attempt burns its window, whatever the outcome. An attempt bound to another
    /// window leaves the current one intact.
    pub fn attempt(&mut self, window: u64, code: &PairingCode, now: Instant) -> Attempt<T> {
        if self.window.as_ref().is_none_or(|w| w.id != window) {
            return Attempt::NoWindow;
        }
        let Some(w) = self.window.take() else {
            return Attempt::NoWindow;
        };
        if now >= w.expires_at {
            return Attempt::NoWindow;
        }
        if bool::from(w.code.as_str().as_bytes().ct_eq(code.as_str().as_bytes())) {
            Attempt::Accepted(w.owner)
        } else {
            Attempt::WrongCode(w.owner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(c: char) -> PairingCode {
        PairingCode::new(c.to_string().repeat(22)).unwrap()
    }

    #[test]
    fn codes_are_random_and_valid() {
        let a = new_code().unwrap();
        let b = new_code().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 22);
    }

    #[test]
    fn right_code_once() {
        let t0 = Instant::now();
        let mut p = Pairing::default();
        let w = p.open(code('A'), t0, "cli").unwrap();
        assert_eq!(p.current(t0), Some(w));
        assert_eq!(p.attempt(w, &code('A'), t0), Attempt::Accepted("cli"));
        assert_eq!(p.current(t0), None);
        assert_eq!(p.attempt(w, &code('A'), t0), Attempt::NoWindow);
    }

    #[test]
    fn wrong_code_burns_window() {
        let t0 = Instant::now();
        let mut p = Pairing::default();
        let w = p.open(code('A'), t0, 1).unwrap();
        assert_eq!(p.attempt(w, &code('B'), t0), Attempt::WrongCode(1));
        assert_eq!(p.attempt(w, &code('A'), t0), Attempt::NoWindow);
    }

    #[test]
    fn expiry() {
        let t0 = Instant::now();
        let mut p = Pairing::default();
        let w = p.open(code('A'), t0, ()).unwrap();
        let late = t0 + WINDOW_TTL;
        assert_eq!(p.current(late), None);
        assert_eq!(p.attempt(w, &code('A'), late), Attempt::NoWindow);
    }

    #[test]
    fn one_window_at_a_time() {
        let t0 = Instant::now();
        let mut p = Pairing::default();
        let first = p.open(code('A'), t0, 1).unwrap();
        assert_eq!(p.open(code('B'), t0, 2), Err(Busy));
        let second = p.open(code('B'), t0 + WINDOW_TTL, 2).unwrap();
        p.close(first);
        assert_eq!(p.current(t0 + WINDOW_TTL), Some(second));
        p.close(second);
        assert_eq!(p.current(t0 + WINDOW_TTL), None);
    }

    #[test]
    fn attempt_from_an_older_window_does_not_burn_the_current_one() {
        let t0 = Instant::now();
        let mut p = Pairing::default();
        let first = p.open(code('A'), t0, 1).unwrap();
        p.close(first);
        let second = p.open(code('B'), t0, 2).unwrap();
        assert_eq!(p.attempt(first, &code('B'), t0), Attempt::NoWindow);
        assert_eq!(p.current(t0), Some(second));
        assert_eq!(p.attempt(second, &code('B'), t0), Attempt::Accepted(2));
    }
}
