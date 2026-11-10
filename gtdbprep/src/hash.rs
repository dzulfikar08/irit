//! SHA-256 for files and bundles, and GTDB-Tk's own SHA-1 folder hash (`tools.sha1_dir`).

use anyhow::{Context, Result};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0xf) as usize] as char);
    }
    s
}

fn feed<D: Digest>(hasher: &mut D, path: &Path) -> Result<u64> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut buf = vec![0u8; 1 << 16];
    let mut total = 0u64;
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            return Ok(total);
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
}

/// SHA-256 and size of one file.
pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    let mut h = Sha256::new();
    let n = feed(&mut h, path)?;
    Ok((hex(&h.finalize()), n))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Files under `root` in the order GTDB-Tk's `sha1_dir` reads them: Python's `os.walk` without
/// following directory symlinks, every file (symlinks to files included), sorted by full path.
/// Paths are relative to `root`, with `/` separators.
pub fn walk_files(root: &Path) -> Result<Vec<String>> {
    fn rec(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type()?;
            let is_dir = if ft.is_symlink() {
                // os.walk lists a symlinked directory but does not descend into it.
                if fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
                    continue;
                }
                false
            } else {
                ft.is_dir()
            };
            if is_dir {
                rec(root, &path, out)?;
            } else {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    rec(root, root, &mut out)?;
    // All full paths share the `root/` prefix, so sorting the relative paths gives Python's order
    // (code-point order equals UTF-8 byte order).
    out.sort();
    Ok(out)
}

/// GTDB-Tk's `sha1_dir`: SHA-1 over the contents of every file, in `walk_files` order. This is
/// what `REF_HASHES` in `gtdbtk/config/common.py` records for each reference folder.
pub fn sha1_dir(root: &Path) -> Result<String> {
    let mut h = Sha1::new();
    for rel in walk_files(root)? {
        feed(&mut h, &root.join(&rel))?;
    }
    Ok(hex(&h.finalize()))
}

/// A SHA-256 over a whole folder that also covers file names: SHA-256 of the text
/// `"<sha256>  <relative path>\n"` for every file in `walk_files` order (the `sha256sum` format).
/// Returns the digest and the file count.
pub fn sha256_tree(root: &Path) -> Result<(String, usize)> {
    let files = walk_files(root)?;
    let mut listing = String::new();
    for rel in &files {
        let (sum, _) = sha256_file(&root.join(rel))?;
        listing.push_str(&format!("{sum}  {rel}\n"));
    }
    Ok((sha256_bytes(listing.as_bytes()), files.len()))
}

/// Recursively list regular files under `root` as absolute paths (no ordering guarantee).
pub fn all_files(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(walk_files(root)?
        .into_iter()
        .map(|r| root.join(r))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_file_matches_bytes() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, b"abc").unwrap();
        let (sum, n) = sha256_file(&p).unwrap();
        assert_eq!(n, 3);
        assert_eq!(sum, sha256_bytes(b"abc"));
    }

    #[test]
    fn sha1_dir_is_concatenation_in_sorted_path_order() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("b")).unwrap();
        fs::write(d.path().join("b/x"), b"2").unwrap();
        fs::write(d.path().join("a"), b"1").unwrap();
        fs::write(d.path().join("c"), b"3").unwrap();
        assert_eq!(walk_files(d.path()).unwrap(), vec!["a", "b/x", "c"]);
        // Same as hashlib.sha1(b"123").hexdigest()
        assert_eq!(
            sha1_dir(d.path()).unwrap(),
            "40bd001563085fc35165329ea1ff5c5ecbdbbeef"
        );
    }

    #[cfg(unix)]
    #[test]
    fn sha1_dir_skips_symlinked_dirs_like_os_walk() {
        let d = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("hidden"), b"zzz").unwrap();
        fs::write(d.path().join("a"), b"1").unwrap();
        std::os::unix::fs::symlink(other.path(), d.path().join("linkdir")).unwrap();
        std::os::unix::fs::symlink(d.path().join("a"), d.path().join("linkfile")).unwrap();
        assert_eq!(walk_files(d.path()).unwrap(), vec!["a", "linkfile"]);
    }

    #[test]
    fn tree_hash_depends_on_names() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        fs::write(a.path().join("x"), b"1").unwrap();
        fs::write(b.path().join("y"), b"1").unwrap();
        assert_eq!(sha1_dir(a.path()).unwrap(), sha1_dir(b.path()).unwrap());
        assert_ne!(
            sha256_tree(a.path()).unwrap().0,
            sha256_tree(b.path()).unwrap().0
        );
    }
}
