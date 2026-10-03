use std::os::raw::c_char;
use std::ptr;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, OSStatus, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use security_framework::base::Error;
use security_framework::os::macos::access::SecAccess;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;
use security_framework_sys::base::{SecAccessRef, errSecDuplicateItem, errSecItemNotFound};
use security_framework_sys::item::{
    kSecAttrAccount, kSecAttrLabel, kSecAttrService, kSecClass, kSecClassGenericPassword,
    kSecUseKeychain, kSecValueData,
};
use security_framework_sys::keychain_item::SecItemAdd;
use zeroize::Zeroizing;

pub const SERVICE: &str = "dev.rbstp.collied.apns";

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecAttrAccess: CFStringRef;
    fn SecTrustedApplicationCreateFromPath(path: *const c_char, app: *mut CFTypeRef) -> OSStatus;
    fn SecAccessCreate(
        descriptor: CFStringRef,
        trusted: CFArrayRef,
        access: *mut SecAccessRef,
    ) -> OSStatus;
}

fn cvt(status: OSStatus) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::from_code(status))
    }
}

// A null path means the calling executable, recorded with its designated requirement:
// a build signed with the same Developer ID identity and identifier keeps access, any
// other program prompts or is refused.
fn this_executable_only() -> Result<SecAccess, Error> {
    let mut app: CFTypeRef = ptr::null();
    // SAFETY: out-pointer is valid; on success the app is returned under the create rule.
    cvt(unsafe { SecTrustedApplicationCreateFromPath(ptr::null(), &mut app) })?;
    // SAFETY: success returned a retained, non-null SecTrustedApplicationRef.
    let app = unsafe { CFType::wrap_under_create_rule(app) };
    let trusted = CFArray::from_CFTypes(&[app]);
    let descriptor = CFString::new(SERVICE);
    let mut access: SecAccessRef = ptr::null_mut();
    // SAFETY: arguments are live CF objects and the out-pointer is valid.
    cvt(unsafe {
        SecAccessCreate(
            descriptor.as_concrete_TypeRef(),
            trusted.as_concrete_TypeRef(),
            &mut access,
        )
    })?;
    // SAFETY: success returned a retained, non-null SecAccessRef.
    Ok(unsafe { SecAccess::wrap_under_create_rule(access) })
}

#[derive(Debug, PartialEq)]
pub enum Stored {
    Added,
    AlreadyPresent,
}

/// `keychain` None is the default (login) keychain.
pub fn store(
    keychain: Option<&SecKeychain>,
    account: &str,
    secret: &[u8],
) -> Result<Stored, Error> {
    let access = this_executable_only()?;
    // SAFETY: the kSec* constants are immutable CFStrings exported by Security.framework.
    let key = |k: CFStringRef| unsafe { CFString::wrap_under_get_rule(k) };
    let mut pairs: Vec<(CFString, CFType)> = vec![
        (
            key(unsafe { kSecClass }),
            key(unsafe { kSecClassGenericPassword }).into_CFType(),
        ),
        (
            key(unsafe { kSecAttrService }),
            CFString::new(SERVICE).into_CFType(),
        ),
        (
            key(unsafe { kSecAttrAccount }),
            CFString::new(account).into_CFType(),
        ),
        (
            key(unsafe { kSecAttrLabel }),
            CFString::new(SERVICE).into_CFType(),
        ),
        (
            key(unsafe { kSecValueData }),
            CFData::from_buffer(secret).into_CFType(),
        ),
        (key(unsafe { kSecAttrAccess }), access.into_CFType()),
    ];
    if let Some(k) = keychain {
        pairs.push((key(unsafe { kSecUseKeychain }), k.as_CFType()));
    }
    let attributes = CFDictionary::from_CFType_pairs(&pairs);
    // SAFETY: attributes is a live dictionary; no result is requested.
    match unsafe { SecItemAdd(attributes.as_concrete_TypeRef(), ptr::null_mut()) } {
        0 => Ok(Stored::Added),
        s if s == errSecDuplicateItem => Ok(Stored::AlreadyPresent),
        s => Err(Error::from_code(s)),
    }
}

/// `keychain` None searches the user's keychain search list.
pub fn read(
    keychain: Option<&SecKeychain>,
    account: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    match find_generic_password(keychain.map(std::slice::from_ref), SERVICE, account) {
        Ok((secret, _)) => Ok(Some(Zeroizing::new(secret.to_vec()))),
        Err(e) if e.code() == errSecItemNotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;

    fn security(args: &[&str]) -> String {
        let out = Command::new("/usr/bin/security")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "security {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    struct TempKeychain {
        path: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl TempKeychain {
        fn create() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("collied-test.keychain-db");
            security(&[
                "create-keychain",
                "-p",
                "collied-test",
                path.to_str().unwrap(),
            ]);
            Self { path, _dir: dir }
        }

        fn open(&self) -> SecKeychain {
            let mut k = SecKeychain::open(&self.path).unwrap();
            k.unlock(Some("collied-test")).unwrap();
            k
        }
    }

    impl Drop for TempKeychain {
        fn drop(&mut self) {
            let _ = Command::new("/usr/bin/security")
                .args(["delete-keychain", self.path.to_str().unwrap()])
                .output();
        }
    }

    // `security dump-keychain -a` entries: (authorizations, applications or None for any).
    fn acl(path: &Path) -> Vec<(String, Option<Vec<String>>)> {
        let dump = security(&["dump-keychain", "-a", path.to_str().unwrap()]);
        let mut entries: Vec<(String, Option<Vec<String>>)> = Vec::new();
        for line in dump.lines().map(str::trim) {
            if let Some(a) = line.strip_prefix("authorizations ") {
                let names = a.split_once(": ").map_or("", |(_, n)| n);
                entries.push((names.to_owned(), Some(Vec::new())));
            } else if line == "applications: <null>" {
                entries.last_mut().unwrap().1 = None;
            } else if let Some((n, app)) = line.split_once(": ")
                && n.parse::<u32>().is_ok()
            {
                let apps = entries.last_mut().unwrap().1.as_mut().unwrap();
                apps.push(app.trim_end_matches(" (OK)").to_owned());
            }
        }
        entries
    }

    #[test]
    fn round_trip_on_a_temporary_keychain() {
        let _no_prompt = SecKeychain::disable_user_interaction().unwrap();
        let temp = TempKeychain::create();
        let kc = temp.open();
        let secret = crate::push::tests::TEST_KEY.as_bytes();

        assert_eq!(read(Some(&kc), "ABCDE12345").unwrap(), None);
        assert_eq!(
            store(Some(&kc), "ABCDE12345", secret).unwrap(),
            Stored::Added
        );
        assert_eq!(
            store(Some(&kc), "ABCDE12345", b"other").unwrap(),
            Stored::AlreadyPresent
        );
        assert_eq!(
            read(Some(&kc), "ABCDE12345").unwrap().unwrap().as_slice(),
            secret
        );
        assert_eq!(read(Some(&kc), "ZZZZZ99999").unwrap(), None);

        let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
        let entries = acl(&temp.path);
        let decrypt: Vec<_> = entries
            .iter()
            .filter(|(auth, _)| auth.split(' ').any(|a| a == "decrypt"))
            .collect();
        assert_eq!(decrypt.len(), 1, "{entries:?}");
        assert_eq!(
            decrypt[0].1.as_deref(),
            Some([exe.display().to_string()].as_slice()),
            "{entries:?}"
        );
        for (auth, apps) in &entries {
            assert!(apps.is_some() || auth == "encrypt", "{entries:?}");
        }
    }
}
