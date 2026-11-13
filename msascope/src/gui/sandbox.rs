//! The macOS App Sandbox (the Mac App Store edition).
//!
//! A sandboxed app may only read files the user chose in an open panel, dropped on the window, or
//! opened from Finder, and only until it quits. To reopen recent files after a relaunch, msascope
//! keeps a security-scoped bookmark for each one, and for folders the user allowed (so a tree saved
//! next to an alignment can be found), and resolves them at start.
//!
//! Outside the sandbox (the dmg, Linux) none of this is needed and every function here does nothing.

use std::path::{Path, PathBuf};

/// Whether msascope runs in the App Sandbox: macOS sets `APP_SANDBOX_CONTAINER_ID` for it.
pub fn active() -> bool {
    cfg!(target_os = "macos") && std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// A security-scoped bookmark for `path` (a file or folder the app can read now), base64.
pub fn bookmark(path: &Path) -> Option<String> {
    if !active() {
        return None;
    }
    imp::bookmark(path)
}

/// Resolve bookmarks and keep access to them until the app quits. Returns, for each bookmark that
/// still resolves, the path it now points to and, when macOS says it is stale (the file moved),
/// a fresh bookmark to store instead.
pub fn restore(bookmarks: &[(PathBuf, String)]) -> Vec<(PathBuf, PathBuf, Option<String>)> {
    if !active() {
        return Vec::new();
    }
    bookmarks
        .iter()
        .filter_map(|(old, b)| imp::restore(b).map(|(new, fresh)| (old.clone(), new, fresh)))
        .collect()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSString, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions,
    };

    pub fn bookmark(path: &Path) -> Option<String> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let data = url
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithSecurityScope,
                None,
                None,
            )
            .ok()?;
        Some(STANDARD.encode(data.to_vec()))
    }

    pub fn restore(b64: &str) -> Option<(PathBuf, Option<String>)> {
        let bytes = STANDARD.decode(b64).ok()?;
        let data = NSData::with_bytes(&bytes);
        let mut stale = Bool::NO;
        // SAFETY: `stale` is a valid pointer for the duration of the call.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                &mut stale,
            )
        }
        .ok()?;
        // SAFETY: plain message send; access is kept until the app quits, as the recent list and
        // allowed folders are meant to work for the whole session.
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return None;
        }
        let path = PathBuf::from(url.path()?.to_string());
        let fresh = if stale.as_bool() {
            bookmark(&path)
        } else {
            None
        };
        // Keep the URL (and so the access) alive for the rest of the session.
        std::mem::forget(url);
        Some((path, fresh))
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::{Path, PathBuf};

    pub fn bookmark(_: &Path) -> Option<String> {
        None
    }

    pub fn restore(_: &str) -> Option<(PathBuf, Option<String>)> {
        None
    }
}
